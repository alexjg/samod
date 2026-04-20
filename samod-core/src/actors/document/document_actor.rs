use std::collections::HashMap;
use std::time::Duration;

use automerge::{Automerge, ChangeHash};

use super::SpawnArgs;
// use super::driver::Driver;
use super::io::{DocumentIoResult, DocumentIoTask};
use crate::actors::document::load::{Load, LoadComplete};
use crate::actors::document::on_disk_state::OnDiskState;
use crate::actors::document::peer_doc_connection::{AnnouncePolicy, PeerDocConnection};
use crate::actors::document::phase::loading::Loading;
use crate::actors::document::phase::ready::Ready;
use crate::actors::document::phase::request::{Request, RequestOutcome};
use crate::actors::document::{ActorInput, DocActorResult, WithDocResult};
use crate::actors::messages::{Broadcast, DocMessage, DocToHubMsgPayload, SyncMessage};
use crate::actors::{DocToHubMsg, HubToDocMsg, RunState};
use crate::doc_search::DocSearchPhase;
use crate::io::{IoResult, IoTaskId};
use crate::network::PeerDocState;
use crate::{
    ConnectionId, DocumentActorId, DocumentChanged, DocumentId, PeerId, StorageKey, SyncDirection,
    SyncMessageStat, UnixTimestamp,
};

use super::errors::DocumentError;

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
    /// Current load state
    load_state: Load,
    /// Sync states for each connected peer
    peer_connections: HashMap<ConnectionId, PeerDocConnection>,
    on_disk_state: OnDiskState,
    /// Ongoing policy check tasks
    check_policy_tasks: HashMap<IoTaskId, ConnectionId>,
    phase: Phase,
    run_state: RunState,
    doc: Automerge,
    last_status_update: Option<DocSearchPhase>,
}

#[derive(Debug)]
pub(crate) enum Phase {
    Loading(Loading),
    Requesting(Request),
    Ready(Ready),
}

impl DocumentActor {
    /// Creates a new document actor for the specified document.
    #[tracing::instrument(skip(initial_content, initial_connections))]
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

        let (phase, doc) = if let Some(doc) = initial_content {
            (Phase::Ready(Ready::new()), doc)
        } else {
            (Phase::Loading(Loading::new()), Automerge::new())
        };

        // Enqueue initial load
        let mut load_state = Load::new(document_id.clone());
        load_state.begin();

        let mut actor = Self {
            document_id,
            doc,
            local_peer_id: local_peer_id.clone(),
            id: actor_id,
            load_state,
            check_policy_tasks: HashMap::new(),
            on_disk_state: OnDiskState::new(),
            peer_connections: HashMap::new(),
            run_state: RunState::Running,
            phase,
            last_status_update: None,
        };

        tracing::trace!(?initial_connections, "applying initial connections");
        for (conn_id, (peer_id, msg)) in initial_connections {
            actor.add_connection(conn_id, peer_id);
            if let Some(msg) = msg {
                actor.handle_input(
                    now,
                    ActorInput::HandleDocMessage {
                        connection_id: conn_id,
                        message: msg,
                        received_at: now,
                    },
                    &mut out,
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
            tracing::warn!(actor_id=%self.id, "ignoring message on stopped document actor");
            return Ok(DocActorResult {
                stopped: true,
                ..Default::default()
            });
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
            tracing::warn!(actor_id=%self.id, "ignoring IO completion on stopped document actor");
            let mut result = DocActorResult::new();
            result.stopped = true;
            return Ok(result);
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

    pub fn document(&self) -> &Automerge {
        &self.doc
    }

    fn document_mut(&mut self) -> &mut Automerge {
        &mut self.doc
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
    /// ```rust,no_run
    /// use automerge::{AutomergeError, ObjId, transaction::Transactable, ObjType};
    /// use samod_core::UnixTimestamp;
    /// # let mut actor: samod_core::actors::document::DocumentActor = todo!(); // DocumentActor instance
    /// let now = UnixTimestamp::now();
    /// let result = actor.with_document::<_, ObjId>(now, |doc| {
    ///     doc.transact::<_, _, AutomergeError>(|tx| {
    ///         tx.put_object(automerge::ROOT, "key", ObjType::Text)
    ///     }).unwrap().result
    /// }).unwrap();
    ///
    /// // Get the closure result
    /// let object_id = result.value;
    ///
    /// // Execute any side effects
    /// for io_task in result.actor_result.io_tasks {
    ///     // storage.execute_document_io(io_task);
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
        let mut guard = self.begin_modification()?;

        let closure_result = f(guard.doc());

        let actor_result = guard.commit(now);

        Ok(WithDocResult::with_side_effects(
            closure_result,
            actor_result,
        ))
    }

    /// Begin a modification of the document, returning a guard for safe access.
    ///
    /// In some scenarios it's not possible to express the modifications that
    /// you need to make to a document using the `with_document` method. In
    /// these cases you can use this method to obtain a guard that allows you to
    /// modify the document directly. Once you have finished you _must_ call
    /// `commit` on the guard to apply the changes and generate any necessary
    /// side effects. Failure to do so will result in a panic when the guard is
    /// dropped.
    ///
    /// # Example
    ///
    /// ```rust,no_run
    /// use automerge::{ROOT, AutomergeError, transaction::Transactable};
    /// # use std::error::Error;
    /// # use samod_core::UnixTimestamp;
    /// # fn example() -> Result<(), Box<dyn Error>> {
    /// # let mut actor: samod_core::actors::document::DocumentActor = todo!(); // DocumentActor instance
    ///
    /// let mut guard = actor.begin_modification()?;
    ///
    /// // Make multiple modifications to the document
    /// guard.doc().transact::<_, _, AutomergeError>(|tx| {
    ///     let list_id = tx.put_object(ROOT, "items", automerge::ObjType::List)?;
    ///     tx.insert(&list_id, 0, "first item")?;
    ///     tx.insert(&list_id, 1, "second item")?;
    ///     Ok(())
    /// }).unwrap();
    ///
    /// // Commit the changes and get the side effects
    /// let now = UnixTimestamp::now();
    /// let result = guard.commit(now);
    ///
    /// // Handle any I/O tasks that were generated
    /// for io_task in result.io_tasks {
    ///     // Execute the I/O task with your storage system
    ///     // storage.execute_document_io(io_task);
    /// }
    /// # Ok(())
    /// # }
    /// ```
    pub fn begin_modification(&mut self) -> Result<WithDocGuard<'_>, DocumentError> {
        // Try to access the internal document
        tracing::Span::current().record("local_peer_id", self.local_peer_id.to_string());
        if !self.is_document_ready() {
            return Err(DocumentError::InvalidState(
                "document is not ready".to_string(),
            ));
        }
        Ok(WithDocGuard::new(self))
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
        matches!(self.phase, Phase::Ready(_))
    }

    #[tracing::instrument(skip(self, input, out), fields(local_peer_id=%self.local_peer_id))]
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
                received_at,
            } => {
                match message {
                    DocMessage::Ephemeral(msg) => {
                        out.emit_ephemeral_message(msg.data.clone());
                        // Forward the message to all other connections
                        let targets = self
                            .peer_connections
                            .iter()
                            .filter_map(|(c, conn)| {
                                if conn.peer_id != msg.sender_id {
                                    Some(*c)
                                } else {
                                    None
                                }
                            })
                            .collect();
                        out.send_broadcast(targets, Broadcast::Gossip { msg });
                    }
                    DocMessage::Sync(msg) => {
                        self.handle_sync_message(now, out, connection_id, msg, received_at)
                    }
                };
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
                            if let Phase::Requesting(request) = &mut self.phase {
                                request.announce_policy_changed(conn_id, policy);
                            };
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
                    self.handle_load(now, out, &snapshots, &incrementals);
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
            return;
        }
        if self.run_state == RunState::Stopping {
            if self.on_disk_state.is_flushed() {
                self.run_state = RunState::Stopped;
                out.send_terminated();
                out.stopped = true;
            }
            return;
        }
        if let Phase::Requesting(request) = &mut self.phase
            && let RequestOutcome::Found = request.outcome(&self.doc)
        {
            self.phase = Phase::Ready(Ready::new());
        }
        self.enqueue_announce_policy_checks(out);
        self.generate_sync_messages(now, out);
        self.on_disk_state
            .save_new_changes(out, &self.document_id, &self.doc);
        out.io_tasks.extend(
            self.load_state
                .step()
                .into_iter()
                .map(|s| s.map(DocumentIoTask::Storage)),
        );
        let status = self.doc_status();
        if Some(&status) != self.last_status_update.as_ref() {
            out.update_search_state(status.clone());
            self.last_status_update = Some(status);
        }
        if let Some(new_peer_states) = self.new_peer_states() {
            out.emit_peer_state_changes(new_peer_states);
        }
    }

    pub fn is_stopped(&self) -> bool {
        self.run_state == RunState::Stopped
    }

    pub fn peers(&self) -> HashMap<ConnectionId, PeerDocState> {
        self.peer_connections
            .iter()
            .map(|(k, v)| (*k, v.state().clone()))
            .collect()
    }

    pub fn conn_peer_id(&self, conn_id: ConnectionId) -> Option<PeerId> {
        self.peer_connections
            .get(&conn_id)
            .map(|pc| pc.peer_id.clone())
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

    fn handle_sync_message(
        &mut self,
        now: UnixTimestamp,
        out: &mut DocActorResult,
        connection_id: ConnectionId,
        msg: SyncMessage,
        received_at: UnixTimestamp,
    ) {
        let Some(peer_conn) = self.peer_connections.get_mut(&connection_id) else {
            tracing::warn!(?connection_id, "no sync state found for message");
            return;
        };
        tracing::debug!(?connection_id, peer_id=?peer_conn.peer_id, ?msg, "received msg");

        if let SyncMessage::Request { .. } = msg {
            // Mark the connection as having requested so we know to send them
            // sync messages in the future, even if we haven't received a sync
            // message from them yet
            peer_conn.mark_requested();
        }

        let bytes = match &msg {
            SyncMessage::Request { data } | SyncMessage::Sync { data } => data.len(),
            SyncMessage::DocUnavailable => 0,
        };

        let duration = match &mut self.phase {
            Phase::Loading(loading) => {
                loading.receive_sync_message(connection_id, msg);
                None
            }
            Phase::Requesting(request) => {
                request.receive_message(now, &mut self.doc, peer_conn, msg)
            }
            Phase::Ready(ready) => {
                let heads_before = self.doc.get_heads();
                let duration = ready.receive_sync_message(now, &mut self.doc, peer_conn, msg);
                let heads_after = self.doc.get_heads();
                if heads_before != heads_after {
                    out.emit_doc_changed(heads_after);
                }
                duration
            }
        };

        if let Some(duration) = duration {
            let queue_duration = if now >= received_at {
                now - received_at
            } else {
                Duration::ZERO
            };
            if bytes > 0 {
                out.sync_message_stats.push(SyncMessageStat {
                    connection_id,
                    direction: SyncDirection::Received,
                    bytes,
                    duration,
                    queue_duration,
                });
            }
        }
    }

    pub fn handle_load(
        &mut self,
        now: UnixTimestamp,
        out: &mut DocActorResult,
        snapshots: &HashMap<StorageKey, Vec<u8>>,
        incrementals: &HashMap<StorageKey, Vec<u8>>,
    ) {
        tracing::trace!("handling load");
        for (key, snapshot) in snapshots {
            if let Err(e) = self.doc.load_incremental(snapshot) {
                tracing::warn!(err=?e, %key, "error loading snapshot chunk");
            }
        }
        for (key, incremental) in incrementals {
            if let Err(e) = self.doc.load_incremental(incremental) {
                tracing::warn!(err=?e, %key, "error loading incremental chunk");
            }
        }

        if let Phase::Loading(loading) = &mut self.phase {
            let pending_sync_messages = loading.take_pending_sync_messages();
            if self.doc.get_heads().is_empty() {
                tracing::debug!("no data found on disk, requesting document");
                let request =
                    Request::new(self.document_id.clone(), self.peer_connections.values());
                self.phase = Phase::Requesting(request);
            } else {
                tracing::trace!("load complete, transitioning to ready");
                self.phase = Phase::Ready(Ready::new())
            };

            for (conn_id, msgs) in pending_sync_messages {
                for msg in msgs {
                    self.handle_sync_message(now, out, conn_id, msg, now);
                }
            }
        }
    }

    pub fn generate_sync_messages(&mut self, now: UnixTimestamp, out: &mut DocActorResult) {
        for (conn_id, peer_conn) in &mut self.peer_connections {
            let generated = match &mut self.phase {
                Phase::Ready(ready) => ready.generate_sync_message(now, &mut self.doc, peer_conn),
                Phase::Requesting(request) => request.generate_message(now, &self.doc, peer_conn),
                Phase::Loading(loading) => {
                    out.pending_sync_messages = loading.pending_msg_count();
                    continue;
                }
            };

            if let Some((msg, duration)) = generated {
                let bytes = match &msg {
                    SyncMessage::Request { data } | SyncMessage::Sync { data } => data.len(),
                    SyncMessage::DocUnavailable => 0,
                };
                if bytes > 0 {
                    out.sync_message_stats.push(SyncMessageStat {
                        connection_id: *conn_id,
                        direction: SyncDirection::Generated,
                        bytes,
                        duration,
                        queue_duration: Duration::ZERO,
                    });
                }
                tracing::debug!(?conn_id, peer_id=?peer_conn.peer_id, ?msg, "sending sync msg");
                out.send_sync_message(*conn_id, self.document_id.clone(), msg);
            }
        }
    }

    fn doc_status(&self) -> DocSearchPhase {
        match &self.phase {
            Phase::Loading(_) => DocSearchPhase::Loading,
            Phase::Requesting(request) => {
                let peer_states = request.peer_states().clone();
                DocSearchPhase::Searching(peer_states)
            }
            Phase::Ready(_) => DocSearchPhase::Ready,
        }
    }

    fn new_peer_states(&mut self) -> Option<HashMap<ConnectionId, PeerDocState>> {
        let mut new_states = HashMap::new();
        for (conn_id, peer) in &mut self.peer_connections {
            if let Some(new_state) = peer.pop() {
                new_states.insert(*conn_id, new_state);
            }
        }
        if new_states.is_empty() {
            None
        } else {
            Some(new_states)
        }
    }

    fn add_connection(&mut self, conn_id: ConnectionId, peer_id: PeerId) {
        assert!(
            !self.peer_connections.contains_key(&conn_id),
            "Connection ID already exists"
        );
        self.peer_connections
            .insert(conn_id, PeerDocConnection::new(peer_id, conn_id));
        if let Phase::Requesting(request) = &mut self.phase
            && let Some(conn) = self.peer_connections.get(&conn_id)
        {
            request.add_connection(conn);
        }
    }

    fn remove_connection(&mut self, conn_id: ConnectionId) {
        self.peer_connections.remove(&conn_id);
        if let Phase::Requesting(request) = &mut self.phase {
            request.remove_connection(conn_id);
        }
    }
}

enum DocGuardState<'a> {
    Modifying {
        actor: &'a mut DocumentActor,
        old_heads: Vec<ChangeHash>,
    },
    Complete,
}

/// The guard returned by [`DocumentActor::begin_modification`]
///
/// This guard provides mutable access to the document. Once you have finished
/// modifying you MUST call [`commit`](WithDocGuard::commit), otherwise a panic
/// will occur when the guard is dropped.
pub struct WithDocGuard<'a> {
    state: DocGuardState<'a>,
}

impl<'a> WithDocGuard<'a> {
    fn new(doc: &'a mut DocumentActor) -> Self {
        let old_heads = doc.document().get_heads();
        Self {
            state: DocGuardState::Modifying {
                actor: doc,
                old_heads,
            },
        }
    }

    /// Returns a mutable reference to the Automerge document.
    pub fn doc(&mut self) -> &mut Automerge {
        match &mut self.state {
            DocGuardState::Modifying {
                actor,
                old_heads: _,
            } => actor.document_mut(),
            DocGuardState::Complete => panic!("Document is already committed"),
        }
    }

    /// Commits the modifications made to the document and returns any side effects.
    pub fn commit(mut self, now: UnixTimestamp) -> DocActorResult {
        let mut out = DocGuardState::Complete;
        std::mem::swap(&mut self.state, &mut out);
        let (actor, old_heads) = match out {
            DocGuardState::Modifying { actor, old_heads } => (actor, old_heads),
            DocGuardState::Complete => {
                // Should never happen as this method takes ownership of the guard
                unreachable!()
            }
        };
        // Check if document was modified and generate side effects
        let new_heads = actor.document().get_heads();

        // Make sure there's one turn of the loop
        let mut actor_result = DocActorResult::new();
        actor.handle_input(now, ActorInput::Tick, &mut actor_result);

        if old_heads != new_heads {
            tracing::debug!(doc_id=%actor.document_id(), "document was modified in actor");
            // Notify main hub that document changed
            actor_result
                .change_events
                .push(DocumentChanged { new_heads });
        }
        actor_result
    }
}

impl<'a> Drop for WithDocGuard<'a> {
    fn drop(&mut self) {
        if let DocGuardState::Modifying { .. } = &mut self.state {
            panic!("WithDocGuard dropped without comitting");
        }
    }
}
