use std::collections::HashMap;

use automerge::Automerge;

use crate::{
    ConnectionId, DocumentId, PeerId, StorageKey, UnixTimestamp,
    actors::{
        RunState,
        messages::{Broadcast, DocMessage, DocToHubMsgPayload, SyncMessage},
    },
    network::PeerDocState,
};

use super::{
    ActorIoAccess, DocumentError, DocumentStatus,
    compaction::{self, SaveState},
    peer_doc_connection::{AnnouncePolicy, PeerDocConnection},
    ready::Ready,
    request::{Request, RequestState},
};

/// The internal state of the async actor runtime.
///
/// This holds the document and any other state needed by the async operations.
#[derive(Debug)]
pub(super) struct ActorState {
    pub phase: Phase,
    /// The document ID
    pub document_id: DocumentId,
    local_peer_id: PeerId,
    doc: Automerge,
    /// Sync states for each connected peer
    peer_connections: HashMap<ConnectionId, PeerDocConnection>,
    save_state: SaveState,
    run_state: RunState,
}

#[derive(Debug)]
pub enum Phase {
    Loading {
        pending_sync_messages: HashMap<ConnectionId, Vec<SyncMessage>>,
    },
    Requesting(Request),
    Ready(Ready),
    NotFound,
}

#[derive(Debug)]
enum PhaseTransition {
    None,
    ToReady,
    ToNotFound,
    ToRequesting(Request),
    ToLoading,
}

impl ActorState {
    pub fn new_loading(document_id: DocumentId, local_peer_id: PeerId, doc: Automerge) -> Self {
        Self {
            phase: Phase::Loading {
                pending_sync_messages: HashMap::new(),
            },
            document_id,
            local_peer_id,
            doc,
            peer_connections: HashMap::new(),
            save_state: SaveState::new(),
            run_state: RunState::Running,
        }
    }

    pub fn new_ready(document_id: DocumentId, local_peer_id: PeerId, doc: Automerge) -> Self {
        Self {
            phase: Phase::Ready(Ready::new()),
            document_id,
            local_peer_id,
            doc,
            peer_connections: HashMap::new(),
            save_state: SaveState::new(),
            run_state: RunState::Running,
        }
    }

    fn handle_phase_transition(&mut self, io: ActorIoAccess, transition: PhaseTransition) {
        match transition {
            PhaseTransition::None => {}
            PhaseTransition::ToReady => {
                tracing::trace!("transitioning to ready");
                io.send_message(DocToHubMsgPayload::DocumentStatusChanged {
                    new_status: DocumentStatus::Ready,
                });
                io.emit_doc_changed(self.doc.get_heads());
                self.phase = Phase::Ready(Ready::new());
            }
            PhaseTransition::ToNotFound => {
                tracing::trace!("transitioning to NotFound");
                io.send_message(DocToHubMsgPayload::DocumentStatusChanged {
                    new_status: DocumentStatus::NotFound,
                });
                if let Phase::Requesting(request) = &self.phase {
                    for peer in request.peers_waiting_for_us_to_respond() {
                        io.send_message(DocToHubMsgPayload::SendSyncMessage {
                            connection_id: peer,
                            document_id: self.document_id.clone(),
                            message: SyncMessage::DocUnavailable,
                        });
                    }
                }
                self.phase = Phase::NotFound;
            }
            PhaseTransition::ToRequesting(request) => {
                tracing::trace!("transitioning to requesting");
                io.send_message(DocToHubMsgPayload::DocumentStatusChanged {
                    new_status: DocumentStatus::Requesting,
                });
                self.phase = Phase::Requesting(request);
            }
            PhaseTransition::ToLoading => {
                tracing::trace!("transitioning to loading");
                io.send_message(DocToHubMsgPayload::DocumentStatusChanged {
                    new_status: DocumentStatus::Loading,
                });
                self.phase = Phase::Loading {
                    pending_sync_messages: HashMap::new(),
                };
            }
        }
    }

    fn check_request_completion(&self) -> PhaseTransition {
        if let Phase::Requesting(request) = &self.phase {
            let RequestState { finished, found } = request.status(&self.doc);
            if finished {
                if found {
                    PhaseTransition::ToReady
                } else {
                    PhaseTransition::ToNotFound
                }
            } else {
                PhaseTransition::None
            }
        } else {
            PhaseTransition::None
        }
    }

    pub fn handle_load(
        &mut self,
        now: UnixTimestamp,
        io: ActorIoAccess,
        snapshots: HashMap<StorageKey, Vec<u8>>,
        incrementals: HashMap<StorageKey, Vec<u8>>,
    ) {
        tracing::trace!("handling load");
        for (key, snapshot) in &snapshots {
            if let Err(e) = self.doc.load_incremental(snapshot) {
                tracing::warn!(err=?e, %key, "error loading snapshot chunk");
            }
        }
        for (key, incremental) in &incrementals {
            if let Err(e) = self.doc.load_incremental(incremental) {
                tracing::warn!(err=?e, %key, "error loading incremental chunk");
            }
        }
        self.save_state
            .add_on_disk(snapshots.into_keys().chain(incrementals.into_keys()));
        if matches!(self.phase, Phase::Loading { .. }) {
            if self.doc.get_heads().is_empty() {
                let eligible_conns = self
                    .peer_connections
                    .values()
                    .any(|p| p.announce_policy() != AnnouncePolicy::DontAnnounce);
                if !eligible_conns {
                    tracing::debug!(
                        "no data found on disk and no connections available, transitioning to NotFound"
                    );
                    self.handle_phase_transition(io, PhaseTransition::ToNotFound);
                } else {
                    tracing::debug!(
                        "no data found on disk but connections available, requesting document"
                    );
                    // We still don't have the doc, request it
                    let mut next_phase = Phase::Requesting(Request::new(
                        self.document_id.clone(),
                        self.peer_connections.values(),
                    ));
                    std::mem::swap(&mut self.phase, &mut next_phase);
                    let Phase::Loading {
                        pending_sync_messages,
                    } = next_phase
                    else {
                        unreachable!("we already checked");
                    };
                    for (conn_id, msgs) in pending_sync_messages {
                        for msg in msgs {
                            self.handle_sync_message(now, io.clone(), conn_id, msg);
                        }
                    }
                    io.send_message(DocToHubMsgPayload::DocumentStatusChanged {
                        new_status: DocumentStatus::Requesting,
                    });
                }
                return;
            }

            tracing::trace!("load complete, transitioning to ready");

            let mut next_phase = Phase::Ready(Ready::new());
            std::mem::swap(&mut self.phase, &mut next_phase);
            let Phase::Loading {
                pending_sync_messages,
            } = next_phase
            else {
                unreachable!("we already checked");
            };
            io.send_message(DocToHubMsgPayload::DocumentStatusChanged {
                new_status: DocumentStatus::Ready,
            });
            for (conn_id, msgs) in pending_sync_messages {
                for msg in msgs {
                    self.handle_sync_message(now, io.clone(), conn_id, msg);
                }
            }
        }
    }

    pub fn add_connection(&mut self, conn_id: ConnectionId, peer_id: PeerId) {
        assert!(
            !self.peer_connections.contains_key(&conn_id),
            "Connection ID already exists"
        );
        let conn = self
            .peer_connections
            .entry(conn_id)
            .insert_entry(PeerDocConnection::new(peer_id, conn_id));
        if let Phase::Requesting(request) = &mut self.phase {
            request.add_connection(conn.get())
        }
    }

    pub fn remove_connection(&mut self, conn_id: ConnectionId) {
        self.peer_connections.remove(&conn_id);
        if let Phase::Requesting(request) = &mut self.phase {
            request.remove_connection(conn_id)
        }
    }

    pub fn handle_doc_message(
        &mut self,
        now: UnixTimestamp,
        io: ActorIoAccess,
        connection_id: ConnectionId,
        msg: DocMessage,
    ) {
        match msg {
            DocMessage::Ephemeral(msg) => {
                io.emit_ephemeral_message(msg.data.clone());
                // Forward the message to all other connections
                let targets = self.broadcast_targets(vec![msg.sender_id.clone()]);
                io.send_message(DocToHubMsgPayload::Broadcast {
                    connections: targets,
                    msg: Broadcast::Gossip { msg },
                });
            }
            DocMessage::Sync(msg) => self.handle_sync_message(now, io, connection_id, msg),
        };
    }

    fn handle_sync_message(
        &mut self,
        now: UnixTimestamp,
        io: ActorIoAccess,
        connection_id: ConnectionId,
        msg: SyncMessage,
    ) {
        let Some(peer_conn) = self.peer_connections.get_mut(&connection_id) else {
            tracing::warn!(?connection_id, "no sync state found for message");
            return;
        };
        tracing::debug!(?connection_id, peer_id=?peer_conn.peer_id, ?msg, "received msg");

        let transition = match &mut self.phase {
            Phase::Loading {
                pending_sync_messages,
            } => {
                pending_sync_messages
                    .entry(connection_id)
                    .or_default()
                    .push(msg);
                PhaseTransition::None
            }
            Phase::Requesting(request) => {
                request.receive_message(now, &mut self.doc, peer_conn, msg);
                self.check_request_completion()
            }
            Phase::Ready(ready) => {
                let heads_before = self.doc.get_heads();
                ready.receive_sync_message(now, &mut self.doc, peer_conn, msg);
                let heads_after = self.doc.get_heads();
                if heads_before != heads_after {
                    io.emit_doc_changed(heads_after);
                }
                PhaseTransition::None
            }
            Phase::NotFound => match msg {
                SyncMessage::Request { data } => {
                    tracing::trace!("received request whilst in notfound, restarting request");
                    let request =
                        Request::new(self.document_id.clone(), self.peer_connections.values());
                    // Apply transition first, then reprocess the message
                    self.handle_phase_transition(
                        io.clone(),
                        PhaseTransition::ToRequesting(request),
                    );
                    self.handle_sync_message(now, io, connection_id, SyncMessage::Request { data });
                    return;
                }
                SyncMessage::Sync { data } => {
                    tracing::trace!("received sync whilst in notfound, moving to ready");
                    // Apply transition first, then reprocess the message
                    self.handle_phase_transition(io.clone(), PhaseTransition::ToReady);
                    self.handle_sync_message(now, io, connection_id, SyncMessage::Sync { data });
                    return;
                }
                SyncMessage::DocUnavailable => PhaseTransition::None,
            },
        };

        self.handle_phase_transition(io, transition);
    }

    pub fn generate_sync_messages(
        &mut self,
        now: UnixTimestamp,
    ) -> HashMap<ConnectionId, Vec<SyncMessage>> {
        let mut result: HashMap<ConnectionId, Vec<SyncMessage>> = HashMap::new();
        for (conn_id, peer_conn) in &mut self.peer_connections {
            match &mut self.phase {
                Phase::Loading { .. } | Phase::NotFound => {}
                Phase::Requesting(request) => {
                    if let Some(msg) = request.generate_message(now, &self.doc, peer_conn) {
                        tracing::debug!(?conn_id, peer_id=?peer_conn.peer_id, ?msg, "sending sync msg");
                        result.entry(*conn_id).or_default().push(msg);
                    }
                }
                Phase::Ready(ready) => {
                    if let Some(msg) = ready.generate_sync_message(now, &mut self.doc, peer_conn) {
                        tracing::debug!(?conn_id, peer_id=?peer_conn.peer_id, ?msg, "sending sync msg");
                        result.entry(*conn_id).or_default().push(msg);
                    }
                }
            }
        }
        result
    }

    pub fn broadcast_targets(&self, omitting_peer_ids: Vec<PeerId>) -> Vec<ConnectionId> {
        self.peer_connections
            .iter()
            .filter(|(_conn_id, conn)| !omitting_peer_ids.contains(&conn.peer_id))
            .map(|(conn_id, _)| *conn_id)
            .collect()
    }

    pub fn ensure_request(&mut self, io: ActorIoAccess) {
        if let Phase::NotFound = self.phase {
            tracing::debug!("reloading document actor");

            // We re-load, then re-request. This means we need to reset all of our sync states
            for peer_conn in self.peer_connections.values_mut() {
                peer_conn.reset_sync_state();
            }
            self.handle_phase_transition(io, PhaseTransition::ToLoading);
        }
    }

    pub fn document(&mut self) -> Result<&mut automerge::Automerge, DocumentError> {
        if let Phase::Ready(_) = self.phase {
            Ok(&mut self.doc)
        } else {
            Err(DocumentError::DocumentNotReady)
        }
    }

    pub fn is_loading(&self) -> bool {
        matches!(self.phase, Phase::Loading { .. })
    }

    pub fn local_peer_id(&self) -> &PeerId {
        &self.local_peer_id
    }

    pub fn pop_new_jobs(&mut self) -> Vec<compaction::Job> {
        self.save_state.pop_new_jobs(&self.document_id, &self.doc)
    }

    pub fn mark_job_complete(&mut self, completion: compaction::JobComplete) {
        self.save_state.mark_job_complete(completion);
    }

    pub fn pop_new_peer_states(&mut self) -> Option<HashMap<ConnectionId, PeerDocState>> {
        let states = self
            .peer_connections
            .iter_mut()
            .filter_map(|(conn_id, conn)| conn.pop().map(|state| (*conn_id, state)))
            .collect::<HashMap<_, _>>();
        if states.is_empty() {
            None
        } else {
            Some(states)
        }
    }

    pub fn pop_announce_policy_tasks(&mut self) -> Vec<(PeerId, ConnectionId)> {
        let mut tasks = Vec::new();
        for peer_conn in self.peer_connections.values_mut() {
            if peer_conn.announce_policy() == AnnouncePolicy::Unknown {
                tasks.push((peer_conn.peer_id.clone(), peer_conn.connection_id));
                peer_conn.set_announce_policy(AnnouncePolicy::Loading);
            }
        }
        tasks
    }

    pub fn set_announce_policy(
        &mut self,
        io: ActorIoAccess,
        connection_id: ConnectionId,
        policy: AnnouncePolicy,
    ) {
        let Some(peer_conn) = self.peer_connections.get_mut(&connection_id) else {
            tracing::warn!(?connection_id, "set_announce_policy for unknown connection");
            return;
        };

        peer_conn.set_announce_policy(policy);

        let transition = if let Phase::Requesting(request) = &mut self.phase {
            request.announce_policy_changed(connection_id, policy);
            self.check_request_completion()
        } else {
            PhaseTransition::None
        };

        self.handle_phase_transition(io, transition);
    }

    pub fn run_state(&self) -> RunState {
        self.run_state
    }

    pub fn set_run_state(&mut self, run_state: RunState) {
        self.run_state = run_state;
    }
}
