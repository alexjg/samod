use std::sync::{Arc, Mutex};

use automerge::Automerge;
use tracing::Instrument;

use super::SpawnArgs;
use super::actor_state::Phase;
use super::run::ActorOutput;
// use super::driver::Driver;
use super::io::{DocumentIoResult, DocumentIoTask};
use crate::actors::document::run::actor_run;
use crate::actors::driver::{Actor, Driver, StepResult};
use crate::actors::messages::{Broadcast, DocToHubMsgPayload};
use crate::actors::{DocToHubMsg, HubToDocMsg, RunState};
use crate::io::{IoResult, IoTask};
use crate::{DocumentActorId, DocumentChanged, DocumentId, UnixTimestamp};

use super::{actor_state::ActorState, errors::DocumentError, run::ActorInput};

/// A document actor manages a single Automerge document.
///
/// Document actors are passive state machines that:
/// - Handle initialization and termination
/// - Can request I/O operations
/// - Process I/O completions
///
/// All I/O operations are requested through the sans-IO pattern,
/// returning tasks for the caller to execute.
pub struct DocumentActor {
    /// The document this actor manages
    document_id: DocumentId,
    /// The ID of this actor according to the main `Samod` instance
    id: DocumentActorId,
    /// Shared internal state for document access
    state: Arc<Mutex<ActorState>>,

    driver: Driver<Self>,
}

impl Actor for DocumentActor {
    type IoTaskAction = DocumentIoTask;
    type IoResult = DocumentIoResult;
    type StepResults = ActorResult;
    type Output = ActorOutput;
    type Input = ActorInput;
    type Complete = ();

    fn finish_step(
        outputs: Vec<Self::Output>,
        new_io_tasks: Vec<IoTask<Self::IoTaskAction>>,
    ) -> Self::StepResults {
        let mut result = ActorResult::default();
        for output in outputs {
            match output {
                ActorOutput::Message(msg) => {
                    result.outgoing_messages.push(DocToHubMsg(msg));
                }
                ActorOutput::EphemeralMessage(data) => {
                    result.ephemeral_messages.push(data);
                }
                ActorOutput::DocChanged { new_heads } => {
                    result.change_events.push(DocumentChanged { new_heads });
                }
            }
        }
        result.io_tasks.extend(new_io_tasks);
        result
    }
}

impl DocumentActor {
    /// Creates a new document actor for the specified document.
    pub fn new(
        now: UnixTimestamp,
        SpawnArgs {
            local_peer_id,
            actor_id,
            document_id,
            initial_content,
            initial_connections,
        }: SpawnArgs,
    ) -> (Self, ActorResult) {
        let info_span = tracing::info_span!("actor_run", %document_id, %local_peer_id, %actor_id);
        let state = info_span.in_scope(|| {
            let state = if let Some(doc) = initial_content {
                ActorState::new_ready(document_id.clone(), local_peer_id, doc)
            } else {
                ActorState::new_loading(document_id.clone(), local_peer_id, Automerge::new())
            };
            Arc::new(Mutex::new(state))
        });

        let mut driver = Driver::<DocumentActor>::spawn(now, |args| {
            let state = state.clone();
            actor_run(args.now, args.rx_input, args.io, state, initial_connections)
                .instrument(info_span)
        });

        let results = match driver.step(now) {
            StepResult::Suspend(results) => results,
            StepResult::Complete { .. } => panic!("document actor finished unexpectedly"),
        };

        let actor = Self {
            document_id,
            id: actor_id,
            state,
            driver,
        };
        (actor, results)
    }

    /// Processes a message from the hub actor and returns the result.
    pub fn handle_message(
        &mut self,
        now: UnixTimestamp,
        message: HubToDocMsg,
    ) -> Result<ActorResult, DocumentError> {
        self.driver.handle_input(now, ActorInput::from(message.0));
        Ok(self.step(now))
    }

    /// Processes the completion of an I/O operation.
    ///
    /// This forwards IO completions to the appropriate async operation
    /// waiting for the result.
    #[tracing::instrument(
        skip(self, io_result),
        fields(
            local_peer_id=%self.state.lock().unwrap().local_peer_id(),
            document_id=%self.state.lock().unwrap().document_id,
            actor_id=%self.id
        )
    )]
    pub fn handle_io_complete(
        &mut self,
        now: UnixTimestamp,
        io_result: IoResult<DocumentIoResult>,
    ) -> Result<ActorResult, DocumentError> {
        self.driver.handle_io_complete(now, io_result);
        Ok(self.step(now))
    }

    /// Returns the document ID this actor manages.
    pub fn document_id(&self) -> &DocumentId {
        &self.document_id
    }

    /// Provides mutable access to the document with automatic side effect handling.
    ///
    /// The closure receives a mutable reference to the Automerge document. Any modifications
    /// will be detected and appropriate side effects will be generated and returned in the
    /// `WithDocResult`.
    ///
    /// Returns an error if the document is not yet loaded or if there's an internal error.
    ///
    /// # Example
    ///
    /// ```text
    /// let result = actor.with_document(|doc| {
    ///     doc.put_object(automerge::ROOT, "key", "value")
    /// })?;
    ///
    /// // Get the closure result
    /// let object_id = result.value?;
    ///
    /// // Execute any side effects
    /// for io_task in result.actor_result.io_tasks {
    ///     storage.execute_document_io(io_task);
    /// }
    /// ```
    #[tracing::instrument(skip(self, f), fields(local_peer_id=tracing::field::Empty))]
    pub fn with_document<F, R>(
        &mut self,
        now: UnixTimestamp,
        f: F,
    ) -> Result<WithDocResult<R>, DocumentError>
    where
        F: FnOnce(&mut Automerge) -> R,
    {
        // Try to access the internal document
        let mut state = self.state.lock().unwrap();
        tracing::Span::current().record("local_peer_id", state.local_peer_id().to_string());
        let document = state.document()?;

        // Capture document state before modification for change detection
        let old_heads = document.get_heads();

        // Execute closure with mutable document access
        let closure_result = f(document);

        // Check if document was modified and generate side effects
        let new_heads = document.get_heads();

        // Drop the borrow before creating the result
        drop(state);

        // Make sure there's one turn of the loop
        self.driver.handle_input(now, ActorInput::Tick);
        let mut actor_result = self.step(now);

        // TODO: do this in the main event loop, not here
        if old_heads != new_heads {
            // Document was changed - generate side effects
            tracing::debug!("Document was modified in actor {:?}", self.id);

            // Notify main hub that document changed
            actor_result
                .change_events
                .push(DocumentChanged { new_heads });
        }

        Ok(WithDocResult::with_side_effects(
            closure_result,
            actor_result,
        ))
    }

    pub fn broadcast(&mut self, _now: UnixTimestamp, msg: Vec<u8>) -> ActorResult {
        let mut result = ActorResult::default();
        result
            .outgoing_messages
            .push(DocToHubMsg(DocToHubMsgPayload::Broadcast {
                connections: self.state.lock().unwrap().broadcast_targets(Vec::new()),
                msg: Broadcast::New { msg },
            }));
        result
    }

    /// Returns true if the document is loaded and ready for operations.
    pub fn is_document_ready(&self) -> bool {
        matches!(self.state.lock().unwrap().phase, Phase::Ready(_))
    }

    fn step(&mut self, now: UnixTimestamp) -> ActorResult {
        if self.state.lock().unwrap().run_state() == RunState::Stopped {
            panic!("document actor is stopped");
        }
        let mut result = match self.driver.step(now) {
            StepResult::Suspend(results) => results,
            StepResult::Complete { results, .. } => results,
        };

        // TODO: do this in the run loop, not here
        if let Some(new_states) = self.state.lock().unwrap().pop_new_peer_states() {
            result
                .outgoing_messages
                .push(DocToHubMsg(DocToHubMsgPayload::PeerStatesChanged {
                    new_states,
                }));
        }

        result
    }

    pub fn is_stopped(&self) -> bool {
        self.state.lock().unwrap().run_state() == RunState::Stopped
    }
}

/// Result of a document operation that includes both the closure result and any side effects.
///
/// When modifying a document via `with_document`, the operation may generate side effects
/// like storage operations (saving changes) or messages to be sent to connected peers.
/// This type encapsulates both the result of the user's closure and the side effects
/// that need to be executed.
///
/// # Type Parameters
///
/// * `T` - The type returned by the closure passed to `with_document`
///
/// # Example
///
/// ```text
/// let result = actor.with_document(|doc| {
///     // Modify document and return some value
///     doc.put_object(automerge::ROOT, "key", "value").unwrap()
/// })?;
///
/// // Access the closure result
/// let object_id = result.value;
///
/// // Execute any side effects
/// for io_task in result.actor_result.io_tasks {
///     // Execute storage operations
/// }
/// for message in result.actor_result.outgoing_messages {
///     // Send messages to main Samod
/// }
/// ```
#[derive(Debug)]
pub struct WithDocResult<T> {
    /// The result returned by the closure passed to `with_document`
    pub value: T,

    /// Any side effects generated by the document operation (IO tasks, messages, etc.)
    pub actor_result: ActorResult,
}

impl<T> WithDocResult<T> {
    /// Creates a new result with the given value and empty side effects
    pub fn new(value: T) -> Self {
        Self {
            value,
            actor_result: ActorResult::new(),
        }
    }

    /// Creates a new result with the given value and actor result
    pub fn with_side_effects(value: T, actor_result: ActorResult) -> Self {
        Self {
            value,
            actor_result,
        }
    }
}

/// Result of processing a message or I/O completion.
#[derive(Debug)]
pub struct ActorResult {
    /// Document I/O tasks that need to be executed by the caller.
    pub io_tasks: Vec<IoTask<DocumentIoTask>>,
    /// Messages to send back to the main system.
    pub outgoing_messages: Vec<DocToHubMsg>,
    /// New ephemeral messages
    pub ephemeral_messages: Vec<Vec<u8>>,
    /// Change events
    pub change_events: Vec<DocumentChanged>,
}

impl ActorResult {
    /// Creates an empty result.
    pub fn new() -> Self {
        Self {
            io_tasks: Vec::new(),
            outgoing_messages: Vec::new(),
            ephemeral_messages: Vec::new(),
            change_events: Vec::new(),
        }
    }
}

impl Default for ActorResult {
    fn default() -> Self {
        Self::new()
    }
}
