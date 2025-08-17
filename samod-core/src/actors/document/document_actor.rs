use std::collections::HashMap;

use automerge::Automerge;

use super::SpawnArgs;
// use super::driver::Driver;
use super::io::{DocumentIoResult, DocumentIoTask};
use crate::actors::document::load::{Load, LoadComplete};
use crate::actors::document::on_disk_state::OnDiskState;
use crate::actors::document::peer_doc_connection::{AnnouncePolicy, PeerDocConnection};
use crate::actors::document::{ActorInput, DocActorResult, DocumentStatus, WithDocResult};
use crate::actors::messages::{Broadcast, DocToHubMsgPayload};
use crate::actors::{DocToHubMsg, HubToDocMsg, RunState};
use crate::io::{IoResult, IoTaskId};
use crate::{ConnectionId, DocumentActorId, DocumentChanged, DocumentId, PeerId, UnixTimestamp};

use super::{doc_state::DocState, errors::DocumentError};

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
    local_peer_id: PeerId,
    /// Shared internal state for document access
    doc_state: DocState,
    /// Current load state
    load_state: Load,
    /// Sync states for each connected peer
    peer_connections: HashMap<ConnectionId, PeerDocConnection>,
    on_disk_state: OnDiskState,
    /// Ongoing policy check tasks
    check_policy_tasks: HashMap<IoTaskId, ConnectionId>,
    run_state: RunState,
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
    ) -> (Self, DocActorResult) {
        let mut out = DocActorResult::default();

        let state = if let Some(doc) = initial_content {
            // Let the hub know this document is ready immediately if we already have content
            out.send_message(DocToHubMsgPayload::DocumentStatusChanged {
                new_status: DocumentStatus::Ready,
            });
            DocState::new_ready(document_id.clone(), doc)
        } else {
            DocState::new_loading(document_id.clone(), Automerge::new())
        };

        // Enqueue initial load
        let mut load_state = Load::new(document_id.clone());
        load_state.begin();

        let mut actor = Self {
            document_id,
            local_peer_id,
            id: actor_id,
            doc_state: state,
            load_state,
            check_policy_tasks: HashMap::new(),
            on_disk_state: OnDiskState::new(),
            peer_connections: HashMap::new(),
            run_state: RunState::Running,
        };

        tracing::trace!(?initial_connections, "applying initial connections");
        for (conn_id, (peer_id, msg)) in initial_connections {
            actor.add_connection(conn_id, peer_id);
            if let Some(msg) = msg {
                actor.doc_state.handle_doc_message(
                    now,
                    &mut out,
                    conn_id,
                    &mut actor.peer_connections,
                    msg,
                );
            }
        }

        actor.step(now, &mut out);
        (actor, out)
    }

    /// Processes a message from the hub actor and returns the result.
    pub fn handle_message(
        &mut self,
        now: UnixTimestamp,
        message: HubToDocMsg,
    ) -> Result<DocActorResult, DocumentError> {
        if self.run_state == RunState::Stopped {
            panic!("document actor is stopped");
        }
        let mut out = DocActorResult::default();
        self.handle_input(now, ActorInput::from(message.0), &mut out);
        Ok(out)
    }

    /// Processes the completion of an I/O operation.
    ///
    /// This forwards IO completions to the appropriate async operation
    /// waiting for the result.
    #[tracing::instrument(
        skip(self, io_result),
        fields(
            local_peer_id=%self.local_peer_id(),
            document_id=%self.document_id,
            actor_id=%self.id
        )
    )]
    pub fn handle_io_complete(
        &mut self,
        now: UnixTimestamp,
        io_result: IoResult<DocumentIoResult>,
    ) -> Result<DocActorResult, DocumentError> {
        if self.run_state == RunState::Stopped {
            panic!("document actor is stopped");
        }
        let mut result = DocActorResult::new();
        let input = ActorInput::IoComplete(io_result);
        self.handle_input(now, input, &mut result);
        Ok(result)
    }

    /// Returns the document ID this actor manages.
    pub fn document_id(&self) -> &DocumentId {
        &self.document_id
    }

    fn local_peer_id(&self) -> PeerId {
        self.local_peer_id.clone()
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
        tracing::Span::current().record("local_peer_id", self.local_peer_id.to_string());
        if !self.doc_state.is_ready() {
            return Err(DocumentError::InvalidState(
                "document is not ready".to_string(),
            ));
        }
        let document = self.doc_state.document_mut();

        // Capture document state before modification for change detection
        let old_heads = document.get_heads();

        // Execute closure with mutable document access
        let closure_result = f(document);

        // Check if document was modified and generate side effects
        let new_heads = document.get_heads();

        // Make sure there's one turn of the loop
        let mut actor_result = DocActorResult::new();
        self.handle_input(now, ActorInput::Tick, &mut actor_result);

        if old_heads != new_heads {
            tracing::debug!(doc_id=%self.id, "document was modified in actor");
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

    pub fn broadcast(&mut self, _now: UnixTimestamp, msg: Vec<u8>) -> DocActorResult {
        let mut result = DocActorResult::new();
        let broadcast_targets = self.peer_connections.keys().copied().collect();
        result
            .outgoing_messages
            .push(DocToHubMsg(DocToHubMsgPayload::Broadcast {
                connections: broadcast_targets,
                msg: Broadcast::New { msg },
            }));
        result
    }

    /// Returns true if the document is loaded and ready for operations.
    pub fn is_document_ready(&self) -> bool {
        self.doc_state.is_ready()
    }

    fn handle_input(&mut self, now: UnixTimestamp, input: ActorInput, out: &mut DocActorResult) {
        match input {
            ActorInput::Terminate => {
                if self.run_state == RunState::Running {
                    self.run_state = RunState::Stopping;
                }
            }
            ActorInput::HandleDocMessage {
                connection_id,
                message,
            } => {
                self.doc_state.handle_doc_message(
                    now,
                    out,
                    connection_id,
                    &mut self.peer_connections,
                    message,
                );
            }
            ActorInput::NewConnection {
                connection_id,
                peer_id,
            } => {
                self.add_connection(connection_id, peer_id);
            }
            ActorInput::ConnectionClosed { connection_id } => {
                self.remove_connection(connection_id);
            }
            ActorInput::Request => {
                self.load_state.begin();
                self.doc_state
                    .request_if_not_already_available(out, &mut self.peer_connections);
            }
            ActorInput::IoComplete(io_result) => {
                match io_result.payload {
                    DocumentIoResult::Storage(storage_result) => {
                        if self.load_state.has_task(io_result.task_id) {
                            self.load_state
                                .handle_result(io_result.task_id, storage_result);
                        } else if self.on_disk_state.has_task(io_result.task_id) {
                            self.on_disk_state
                                .task_complete(io_result.task_id, storage_result);
                        } else {
                            panic!("unexpected storage result");
                        }
                    }
                    DocumentIoResult::CheckAnnouncePolicy(should_announce) => {
                        let Some(conn_id) = self.check_policy_tasks.remove(&io_result.task_id)
                        else {
                            panic!("unexpected announce policy completion");
                        };
                        let policy = if should_announce {
                            AnnouncePolicy::Announce
                        } else {
                            AnnouncePolicy::DontAnnounce
                        };
                        if let Some(peer_conn) = self.peer_connections.get_mut(&conn_id) {
                            peer_conn.set_announce_policy(policy);
                            self.doc_state.set_announce_policy(out, conn_id, policy);
                        } else {
                            tracing::warn!(
                                ?conn_id,
                                "announce policy check for unknown connection ID",
                            );
                        }
                    }
                }
                if let Some(LoadComplete {
                    snapshots,
                    incrementals,
                }) = self.load_state.take_complete()
                {
                    self.doc_state.handle_load(
                        now,
                        out,
                        &mut self.peer_connections,
                        &snapshots,
                        &incrementals,
                    );
                    self.on_disk_state
                        .add_keys(snapshots.into_keys().chain(incrementals.into_keys()));
                }
            }
            ActorInput::Tick => {}
        }
        self.step(now, out);
    }

    fn step(&mut self, now: UnixTimestamp, out: &mut DocActorResult) {
        if self.run_state == RunState::Stopped {
            panic!("document actor is stopped");
        }
        if self.run_state == RunState::Stopping {
            if self.on_disk_state.is_flushed() {
                self.run_state = RunState::Stopped;
                out.send_message(DocToHubMsgPayload::Terminated);
                out.stopped = true;
            }
            return;
        }
        self.enqueue_announce_policy_checks(out);
        self.generate_sync_messages(now, out);
        self.notify_of_new_peer_states(out);
        self.on_disk_state
            .save_new_changes(out, &self.document_id, self.doc_state.document());
        out.io_tasks.extend(
            self.load_state
                .step()
                .into_iter()
                .map(|s| s.map(DocumentIoTask::Storage)),
        );
    }

    pub fn is_stopped(&self) -> bool {
        self.run_state == RunState::Stopped
    }

    fn enqueue_announce_policy_checks(&mut self, out: &mut DocActorResult) {
        for peer_conn in self.peer_connections.values_mut() {
            if peer_conn.announce_policy() == AnnouncePolicy::Unknown {
                tracing::trace!(
                    peer_id=?peer_conn.peer_id,
                    conn_id=?peer_conn.connection_id,
                    "checking announce policy"
                );
                let task_id = out.check_announce_policy(peer_conn.peer_id.clone());
                self.check_policy_tasks
                    .insert(task_id, peer_conn.connection_id);
                peer_conn.set_announce_policy(AnnouncePolicy::Loading);
            }
        }
    }

    fn generate_sync_messages(&mut self, now: UnixTimestamp, out: &mut DocActorResult) {
        let doc_id = self.document_id.clone();
        for (conn_id, msgs) in self
            .doc_state
            .generate_sync_messages(now, &mut self.peer_connections)
        {
            for msg in msgs {
                out.send_message(DocToHubMsgPayload::SendSyncMessage {
                    connection_id: conn_id,
                    document_id: doc_id.clone(),
                    message: msg,
                });
            }
        }
    }

    fn notify_of_new_peer_states(&mut self, out: &mut DocActorResult) {
        let states = self
            .peer_connections
            .iter_mut()
            .filter_map(|(conn_id, conn)| conn.pop().map(|state| (*conn_id, state)))
            .collect::<HashMap<_, _>>();
        if !states.is_empty() {
            out.send_message(DocToHubMsgPayload::PeerStatesChanged { new_states: states })
        }
    }

    fn add_connection(&mut self, conn_id: ConnectionId, peer_id: PeerId) {
        assert!(
            !self.peer_connections.contains_key(&conn_id),
            "Connection ID already exists"
        );
        let conn = self
            .peer_connections
            .entry(conn_id)
            .insert_entry(PeerDocConnection::new(peer_id, conn_id));
        self.doc_state.add_connection(conn.get());
    }

    fn remove_connection(&mut self, conn_id: ConnectionId) {
        self.peer_connections.remove(&conn_id);
        self.doc_state.remove_connection(conn_id);
    }
}
