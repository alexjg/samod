use std::collections::HashMap;
use std::time::Duration;

use automerge::{Automerge, ChangeHash, ReadDoc, sync};

use crate::{
    ConnectionId, DocumentId, PeerRequestState, ShareConfig, UnixTimestamp,
    actors::{document::peer_doc_connection::PeerDocConnection, messages::SyncMessage},
};

#[derive(Debug)]
pub(crate) struct Request {
    #[allow(dead_code)]
    doc_id: DocumentId,
    peer_states: HashMap<ConnectionId, Peer>,
}

#[derive(Debug)]
struct Peer {
    state: PeerState,
    policy_pending: bool,
}

#[derive(Debug)]
enum PeerState {
    Requesting(Requesting),
    RequestedFromUs {
        unavailable_response_sent: bool,
        blocked_by_policy: bool,
    },
    Unavailable,
    Syncing {
        their_heads: Vec<ChangeHash>,
    },
}

#[derive(Debug)]
enum Requesting {
    AwaitingSend,
    Sent,
    AwaitingSharePolicy,
    BlockedByPolicy,
}

impl From<Option<ShareConfig>> for Requesting {
    fn from(value: Option<ShareConfig>) -> Self {
        match value {
            Some(config) if config.can_request() => Requesting::AwaitingSend,
            Some(_) => Requesting::BlockedByPolicy,
            None => Requesting::AwaitingSharePolicy,
        }
    }
}

/// Outcome of checking request status.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RequestOutcome {
    /// Still searching, peers are being queried
    Searching,
    /// Document found and synced
    Found,
    /// All peers exhausted, no document found (but still in Requesting phase)
    Exhausted,
}

impl Request {
    pub(crate) fn new<'a, I: Iterator<Item = &'a PeerDocConnection>>(
        doc_id: DocumentId,
        connections: I,
    ) -> Self {
        Self {
            peer_states: connections
                .map(|c| {
                    (
                        c.connection_id,
                        Peer {
                            state: PeerState::Requesting(c.share_config().into()),
                            policy_pending: c.share_config().is_none(),
                        },
                    )
                })
                .collect(),
            doc_id,
        }
    }

    pub(crate) fn add_connection(&mut self, conn: &PeerDocConnection) {
        self.peer_states.insert(
            conn.connection_id,
            Peer {
                state: PeerState::Requesting(conn.share_config().into()),
                policy_pending: conn.share_config().is_none(),
            },
        );
    }

    pub(crate) fn remove_connection(&mut self, id: ConnectionId) {
        self.peer_states.remove(&id);
    }

    pub(crate) fn receive_message(
        &mut self,
        now: UnixTimestamp,
        doc: &mut Automerge,
        conn: &mut PeerDocConnection,
        msg: SyncMessage,
    ) -> Option<Duration> {
        let Some(peer) = self.peer_states.get_mut(&conn.connection_id) else {
            tracing::warn!(connection_id=?conn.connection_id, "received message for unknown connection");
            return None;
        };
        // Track the peer's sync state (and advertise our read-only flag), but
        // do not enter Syncing and wait for heads we refuse to apply. Retaining
        // their heads also lets us respond if we later obtain the document
        // elsewhere, even with announcements disabled.
        if !conn.can_write()
            && let SyncMessage::Sync { data } = &msg
        {
            return apply_sync_data(now, doc, conn, peer, data);
        }
        match (msg, &mut peer.state) {
            (SyncMessage::Request { .. }, PeerState::Requesting { .. }) => {
                peer.state = PeerState::RequestedFromUs {
                    unavailable_response_sent: false,
                    blocked_by_policy: !conn.can_write(),
                };
                None
            }
            (SyncMessage::Request { .. }, PeerState::RequestedFromUs { .. }) => None,
            (SyncMessage::Request { .. }, PeerState::Unavailable) => None,
            (SyncMessage::Request { .. }, PeerState::Syncing { .. }) => {
                // This is weird, they sent us a request whilst we're syncing with them. Maybe
                // they restarted? Either way, mark them as unavailable
                peer.state = PeerState::Unavailable;
                None
            }
            (SyncMessage::Sync { data }, PeerState::Requesting { .. }) => {
                let duration = apply_sync_data(now, doc, conn, peer, &data)?;
                let their_heads = conn.their_heads().unwrap_or_default();
                if their_heads.is_empty() {
                    tracing::trace!("their heads are empty, transitioning to unavailable");
                    peer.state = PeerState::Unavailable;
                } else {
                    tracing::info!(connection_id=?conn.connection_id, "starting sync with peer");
                    peer.state = PeerState::Syncing { their_heads };
                }
                Some(duration)
            }
            (
                SyncMessage::Sync { data },
                PeerState::Unavailable | PeerState::RequestedFromUs { .. },
            ) => {
                let duration = apply_sync_data(now, doc, conn, peer, &data)?;
                let their_heads = conn.their_heads().unwrap_or_default();
                peer.state = PeerState::Syncing { their_heads };
                Some(duration)
            }
            (SyncMessage::Sync { data }, PeerState::Syncing { .. }) => {
                apply_sync_data(now, doc, conn, peer, &data)
            }
            (
                SyncMessage::DocUnavailable,
                PeerState::Requesting { .. } | PeerState::RequestedFromUs { .. },
            ) => {
                peer.state = PeerState::Unavailable;
                None
            }
            (SyncMessage::DocUnavailable, PeerState::Unavailable) => None,
            (SyncMessage::DocUnavailable, PeerState::Syncing { .. }) => {
                peer.state = PeerState::Unavailable;
                None
            }
        }
    }

    pub(crate) fn generate_messages(
        &mut self,
        now: UnixTimestamp,
        doc: &Automerge,
        conn: &mut PeerDocConnection,
    ) -> Vec<(SyncMessage, Duration)> {
        if !conn.can_read() {
            return Vec::new();
        }
        if !conn.can_write() && conn.their_heads().is_some() {
            return conn
                .generate_sync_messages(now, doc)
                .into_iter()
                .map(|(msg, duration)| (SyncMessage::Sync { data: msg.encode() }, duration))
                .collect();
        }
        let any_peer_is_syncing = self
            .peer_states
            .values()
            .any(|s| !s.policy_pending && matches!(s.state, PeerState::Syncing { .. }));
        let Some(peer) = self.peer_states.get_mut(&conn.connection_id) else {
            tracing::warn!(conn_id=?conn.connection_id, "no peer state for connection ID");
            return Vec::new();
        };
        match &mut peer.state {
            PeerState::Requesting(Requesting::BlockedByPolicy)
                if conn.can_write() && conn.can_send() =>
            {
                // Resume an inbound sync after a grant, without proactively
                // announcing/requesting a document the peer did not ask about.
                let generated: Vec<_> = conn
                    .generate_sync_messages(now, doc)
                    .into_iter()
                    .map(|(msg, duration)| (SyncMessage::Sync { data: msg.encode() }, duration))
                    .collect();
                if !generated.is_empty() {
                    peer.state = PeerState::Requesting(Requesting::Sent);
                }
                generated
            }
            PeerState::Requesting(requesting) => {
                if !matches!(requesting, Requesting::AwaitingSend) {
                    return Vec::new();
                }
                // If we're already syncing with another peer, don't send a request yet.
                // Otherwise we end up in a situation where in topologies like this:
                //
                // alice <-> bob <-> carol <-> derek
                //
                // Alice could create a document and send it to bob, who then immediately
                // starts requesting the document and sends a request to carol. If derek
                // then requests the document from carol she will send an unavailable
                // response back to derek because from her perspective all connected
                // peers have requested a document she doesn't have
                if any_peer_is_syncing {
                    return Vec::new();
                }
                conn.prepare_request();
                *requesting = Requesting::Sent;
                conn.generate_sync_messages(now, doc)
                    .into_iter()
                    .enumerate()
                    .map(|(index, (msg, duration))| {
                        let data = msg.encode();
                        // Only the first message initiates the request; the
                        // remainder of the batch continues the sync exchange.
                        let msg = if index == 0 {
                            SyncMessage::Request { data }
                        } else {
                            SyncMessage::Sync { data }
                        };
                        (msg, duration)
                    })
                    .collect()
            }
            PeerState::Syncing { .. } => conn
                .generate_sync_messages(now, doc)
                .into_iter()
                .map(|(msg, duration)| (SyncMessage::Sync { data: msg.encode() }, duration))
                .collect(),
            PeerState::Unavailable => Vec::new(),
            PeerState::RequestedFromUs {
                unavailable_response_sent,
                ..
            } => {
                if *unavailable_response_sent {
                    Vec::new()
                } else {
                    *unavailable_response_sent = true;
                    vec![(SyncMessage::DocUnavailable, Duration::default())]
                }
            }
        }
    }

    pub(crate) fn outcome(&self, doc: &Automerge) -> RequestOutcome {
        let all_peers_unavailable = self.peer_states.values().all(|peer| {
            !peer.policy_pending
                && matches!(
                    peer.state,
                    PeerState::Unavailable
                        | PeerState::RequestedFromUs { .. }
                        | PeerState::Requesting(Requesting::BlockedByPolicy)
                )
        });

        let any_sync_is_done = self.peer_states.values().any(|peer| {
            !peer.policy_pending
                && matches!(&peer.state, PeerState::Syncing { their_heads }
                if their_heads.iter().all(|head| doc.get_change_by_hash(head).is_some()))
        });

        if any_sync_is_done {
            RequestOutcome::Found
        } else if all_peers_unavailable && !self.peer_states.is_empty() {
            RequestOutcome::Exhausted
        } else {
            RequestOutcome::Searching
        }
    }

    /// Expose per-peer states for building status updates.
    /// Returns (connection_id, state) for each peer we're tracking.
    pub(crate) fn peer_states(&self) -> HashMap<ConnectionId, PeerRequestState> {
        self.peer_states
            .iter()
            .map(|(conn_id, peer)| {
                let state = if peer.policy_pending {
                    PeerRequestState::Requested
                } else {
                    match &peer.state {
                        PeerState::Requesting(Requesting::BlockedByPolicy)
                        | PeerState::RequestedFromUs {
                            blocked_by_policy: true,
                            ..
                        } => PeerRequestState::BlockedByPolicy,
                        PeerState::Requesting(_) | PeerState::RequestedFromUs { .. } => {
                            PeerRequestState::Requested
                        }
                        PeerState::Unavailable => PeerRequestState::Unavailable,
                        PeerState::Syncing { .. } => PeerRequestState::Syncing,
                    }
                };
                (*conn_id, state)
            })
            .collect()
    }

    pub(crate) fn share_policy_changed(
        &mut self,
        peer: ConnectionId,
        previous: Option<ShareConfig>,
        config: ShareConfig,
    ) {
        let Some(peer) = self.peer_states.get_mut(&peer) else {
            return;
        };
        peer.policy_pending = false;
        if matches!(
            peer.state,
            PeerState::Requesting(Requesting::AwaitingSharePolicy)
        ) {
            peer.state = PeerState::Requesting(Some(config).into());
        } else if previous == Some(config) {
            // An unchanged decision must not restart an in-progress exchange.
        } else if !config.can_write() || previous.is_some_and(|old| !old.can_write()) {
            peer.state = PeerState::Requesting(Some(config).into());
        } else if config.can_request()
            && matches!(
                peer.state,
                PeerState::Requesting(Requesting::BlockedByPolicy)
                    | PeerState::Unavailable
                    | PeerState::RequestedFromUs { .. }
            )
        {
            peer.state = PeerState::Requesting(Requesting::AwaitingSend);
        } else if !config.can_request()
            && matches!(peer.state, PeerState::Requesting(Requesting::AwaitingSend))
        {
            peer.state = PeerState::Requesting(Requesting::BlockedByPolicy);
        }
    }
}

/// Decode and apply sync data to a peer connection, returning the duration
/// of the automerge operation. Returns `None` if decoding or application
/// fails (the peer is marked as unavailable in that case).
fn apply_sync_data(
    now: UnixTimestamp,
    doc: &mut Automerge,
    conn: &mut PeerDocConnection,
    peer: &mut Peer,
    data: &[u8],
) -> Option<Duration> {
    let sync_msg = match sync::Message::decode(data) {
        Ok(msg) => msg,
        Err(e) => {
            tracing::warn!(
                connection_id=?conn.connection_id, err=?e,
                "failed to decode sync message, marking peer as unavailable"
            );
            peer.state = PeerState::Unavailable;
            return None;
        }
    };
    match conn.receive_sync_message(now, doc, sync_msg) {
        Ok(duration) => Some(duration),
        Err(e) => {
            tracing::warn!(
                connection_id=?conn.connection_id, err=?e,
                "failed to apply sync message, marking peer as unavailable"
            );
            peer.state = PeerState::Unavailable;
            None
        }
    }
}
