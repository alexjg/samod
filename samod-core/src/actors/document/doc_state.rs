use std::collections::HashMap;

use automerge::Automerge;

use crate::{
    ConnectionId, DocumentId, StorageKey, UnixTimestamp,
    actors::{
        document::DocActorResult,
        messages::{Broadcast, DocMessage, DocToHubMsgPayload, SyncMessage},
    },
};

use super::{
    DocumentStatus,
    peer_doc_connection::{AnnouncePolicy, PeerDocConnection},
    ready::Ready,
    request::{Request, RequestState},
};

#[derive(Debug)]
pub(super) struct DocState {
    phase: Phase,
    /// The document ID
    document_id: DocumentId,
    doc: Automerge,
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

impl DocState {
    pub fn new_loading(document_id: DocumentId, doc: Automerge) -> Self {
        Self {
            phase: Phase::Loading {
                pending_sync_messages: HashMap::new(),
            },
            document_id,
            doc,
        }
    }

    pub fn new_ready(document_id: DocumentId, doc: Automerge) -> Self {
        Self {
            phase: Phase::Ready(Ready::new()),
            document_id,
            doc,
        }
    }

    fn handle_phase_transition(&mut self, out: &mut DocActorResult, transition: PhaseTransition) {
        match transition {
            PhaseTransition::None => {}
            PhaseTransition::ToReady => {
                tracing::trace!("transitioning to ready");
                out.send_message(DocToHubMsgPayload::DocumentStatusChanged {
                    new_status: DocumentStatus::Ready,
                });
                out.emit_doc_changed(self.doc.get_heads());
                self.phase = Phase::Ready(Ready::new());
            }
            PhaseTransition::ToNotFound => {
                tracing::trace!("transitioning to NotFound");
                out.send_message(DocToHubMsgPayload::DocumentStatusChanged {
                    new_status: DocumentStatus::NotFound,
                });
                if let Phase::Requesting(request) = &self.phase {
                    for peer in request.peers_waiting_for_us_to_respond() {
                        out.send_message(DocToHubMsgPayload::SendSyncMessage {
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
                out.send_message(DocToHubMsgPayload::DocumentStatusChanged {
                    new_status: DocumentStatus::Requesting,
                });
                self.phase = Phase::Requesting(request);
            }
            PhaseTransition::ToLoading => {
                tracing::trace!("transitioning to loading");
                out.send_message(DocToHubMsgPayload::DocumentStatusChanged {
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
        out: &mut DocActorResult,
        peer_connections: &mut HashMap<ConnectionId, PeerDocConnection>,
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
        // self.save_state
        //     .add_on_disk(snapshots.into_keys().chain(incrementals.into_keys()));
        if matches!(self.phase, Phase::Loading { .. }) {
            if self.doc.get_heads().is_empty() {
                let eligible_conns = peer_connections
                    .values()
                    .any(|p| p.announce_policy() != AnnouncePolicy::DontAnnounce);
                if !eligible_conns {
                    tracing::debug!(
                        "no data found on disk and no connections available, transitioning to NotFound"
                    );
                    self.handle_phase_transition(out, PhaseTransition::ToNotFound);
                } else {
                    tracing::debug!(
                        "no data found on disk but connections available, requesting document"
                    );
                    // We still don't have the doc, request it
                    let mut next_phase = Phase::Requesting(Request::new(
                        self.document_id.clone(),
                        peer_connections.values(),
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
                            self.handle_sync_message(now, out, conn_id, peer_connections, msg);
                        }
                    }
                    out.send_message(DocToHubMsgPayload::DocumentStatusChanged {
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
            out.send_message(DocToHubMsgPayload::DocumentStatusChanged {
                new_status: DocumentStatus::Ready,
            });
            for (conn_id, msgs) in pending_sync_messages {
                for msg in msgs {
                    self.handle_sync_message(now, out, conn_id, peer_connections, msg);
                }
            }
        }
    }

    pub(crate) fn add_connection(&mut self, conn: &PeerDocConnection) {
        if let Phase::Requesting(request) = &mut self.phase {
            request.add_connection(conn)
        }
    }

    pub(crate) fn remove_connection(&mut self, conn_id: ConnectionId) {
        if let Phase::Requesting(request) = &mut self.phase {
            request.remove_connection(conn_id)
        }
    }

    pub fn handle_doc_message(
        &mut self,
        now: UnixTimestamp,
        out: &mut DocActorResult,
        connection_id: ConnectionId,
        peer_connections: &mut HashMap<ConnectionId, PeerDocConnection>,
        msg: DocMessage,
    ) {
        match msg {
            DocMessage::Ephemeral(msg) => {
                out.emit_ephemeral_message(msg.data.clone());
                // Forward the message to all other connections
                let targets = peer_connections
                    .iter()
                    .filter_map(|(c, conn)| {
                        if conn.peer_id != msg.sender_id {
                            Some(*c)
                        } else {
                            None
                        }
                    })
                    .collect();
                out.send_message(DocToHubMsgPayload::Broadcast {
                    connections: targets,
                    msg: Broadcast::Gossip { msg },
                });
            }
            DocMessage::Sync(msg) => {
                self.handle_sync_message(now, out, connection_id, peer_connections, msg)
            }
        };
    }

    fn handle_sync_message(
        &mut self,
        now: UnixTimestamp,
        out: &mut DocActorResult,
        connection_id: ConnectionId,
        peer_connections: &mut HashMap<ConnectionId, PeerDocConnection>,
        msg: SyncMessage,
    ) {
        let Some(peer_conn) = peer_connections.get_mut(&connection_id) else {
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
                    out.emit_doc_changed(heads_after);
                }
                PhaseTransition::None
            }
            Phase::NotFound => match msg {
                SyncMessage::Request { data } => {
                    tracing::trace!("received request whilst in notfound, restarting request");
                    let request = Request::new(self.document_id.clone(), peer_connections.values());
                    // Apply transition first, then reprocess the message
                    self.handle_phase_transition(out, PhaseTransition::ToRequesting(request));
                    self.handle_sync_message(
                        now,
                        out,
                        connection_id,
                        peer_connections,
                        SyncMessage::Request { data },
                    );
                    return;
                }
                SyncMessage::Sync { data } => {
                    tracing::trace!("received sync whilst in notfound, moving to ready");
                    // Apply transition first, then reprocess the message
                    self.handle_phase_transition(out, PhaseTransition::ToReady);
                    self.handle_sync_message(
                        now,
                        out,
                        connection_id,
                        peer_connections,
                        SyncMessage::Sync { data },
                    );
                    return;
                }
                SyncMessage::DocUnavailable => PhaseTransition::None,
            },
        };

        self.handle_phase_transition(out, transition);
    }

    pub fn generate_sync_messages(
        &mut self,
        now: UnixTimestamp,
        peer_connections: &mut HashMap<ConnectionId, PeerDocConnection>,
    ) -> HashMap<ConnectionId, Vec<SyncMessage>> {
        let mut result: HashMap<ConnectionId, Vec<SyncMessage>> = HashMap::new();
        for (conn_id, peer_conn) in peer_connections {
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

    /// If the document is in a `NotFound` phase, re-request it from everyone we
    /// know of, otherwise don't do anything
    pub(crate) fn request_if_not_already_available(
        &mut self,
        out: &mut DocActorResult,
        peer_connections: &mut HashMap<ConnectionId, PeerDocConnection>,
    ) {
        if let Phase::NotFound = self.phase {
            tracing::debug!("reloading document actor");

            // We re-load, then re-request. This means we need to reset all of our sync states
            for peer_conn in peer_connections.values_mut() {
                peer_conn.reset_sync_state();
            }
            self.handle_phase_transition(out, PhaseTransition::ToLoading);
        }
    }

    pub(crate) fn is_ready(&self) -> bool {
        matches!(self.phase, Phase::Ready(_))
    }

    pub(crate) fn document_mut(&mut self) -> &mut automerge::Automerge {
        &mut self.doc
    }

    pub(crate) fn document(&self) -> &automerge::Automerge {
        &self.doc
    }

    pub(crate) fn set_announce_policy(
        &mut self,
        out: &mut DocActorResult,
        connection_id: ConnectionId,
        policy: AnnouncePolicy,
    ) {
        let transition = if let Phase::Requesting(request) = &mut self.phase {
            request.announce_policy_changed(connection_id, policy);
            self.check_request_completion()
        } else {
            PhaseTransition::None
        };

        self.handle_phase_transition(out, transition);
    }
}
