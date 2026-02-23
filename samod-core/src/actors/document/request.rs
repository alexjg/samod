use std::collections::HashMap;

use automerge::{Automerge, ChangeHash, ReadDoc, sync};

use crate::{ConnectionId, DocumentId, UnixTimestamp, actors::messages::SyncMessage};

use super::peer_doc_connection::{AnnouncePolicy, PeerDocConnection};

#[derive(Debug)]
pub(crate) struct Request {
    #[allow(dead_code)]
    doc_id: DocumentId,
    peer_states: HashMap<ConnectionId, Peer>,
}

#[derive(Debug)]
struct Peer {
    state: PeerState,
}

#[derive(Debug)]
enum PeerState {
    Requesting(Requesting),
    RequestedFromUs,
    Unavailable,
    Syncing { their_heads: Vec<ChangeHash> },
}

#[derive(Debug)]
enum Requesting {
    AwaitingSend,
    Sent,
    AwaitingAnnouncePolicy,
    NotSentDueToAnnouncePolicy,
}

impl From<AnnouncePolicy> for Requesting {
    fn from(value: AnnouncePolicy) -> Self {
        match value {
            AnnouncePolicy::DontAnnounce => Requesting::NotSentDueToAnnouncePolicy,
            AnnouncePolicy::Announce => Requesting::AwaitingSend,
            AnnouncePolicy::Loading | AnnouncePolicy::Unknown => Requesting::AwaitingAnnouncePolicy,
        }
    }
}

pub(crate) struct RequestState {
    pub(crate) finished: bool,
    pub(crate) found: bool,
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
                            state: PeerState::Requesting(c.announce_policy().into()),
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
                state: PeerState::Requesting(conn.announce_policy().into()),
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
    ) {
        let Some(peer) = self.peer_states.get_mut(&conn.connection_id) else {
            tracing::warn!(connection_id=?conn.connection_id, "received message for unknown connection");
            return;
        };
        match (msg, &mut peer.state) {
            (SyncMessage::Request { .. }, PeerState::Requesting { .. }) => {
                peer.state = PeerState::RequestedFromUs;
            }
            (SyncMessage::Request { .. }, PeerState::RequestedFromUs) => {
                // nothing to do
            }
            (SyncMessage::Request { .. }, PeerState::Unavailable) => {
                // Nothing to do, they're already unavailable
            }
            (SyncMessage::Request { .. }, PeerState::Syncing { .. }) => {
                // This is weird, they sent us a request whilst we're syncing with them. Maybe
                // they restarted? Eithe way, mark them as unavailable
                peer.state = PeerState::Unavailable;
            }
            (SyncMessage::Sync { data }, PeerState::Requesting { .. }) => {
                // They have the document, start syncing it
                let sync_msg = match sync::Message::decode(&data) {
                    Ok(msg) => msg,
                    Err(e) => {
                        tracing::warn!(
                            connection_id=?conn.connection_id, err=?e,
                            "failed to decode sync message, marking peer as unavailable"
                        );
                        peer.state = PeerState::Unavailable;
                        return;
                    }
                };
                if let Err(e) = conn.receive_sync_message(now, doc, sync_msg) {
                    tracing::warn!(
                        connection_id=?conn.connection_id, err=?e,
                        "failed to apply sync message, marking peer as unavailable"
                    );
                    peer.state = PeerState::Unavailable;
                    return;
                }

                let their_heads = conn.their_heads().unwrap_or_default();
                if their_heads.is_empty() {
                    tracing::trace!("their heads are empty, transitioning to unavailable");
                    // If they have no heads, we can consider them unavailable
                    peer.state = PeerState::Unavailable;
                } else {
                    // Otherwise, we can start syncing with them
                    tracing::info!(connection_id=?conn.connection_id, "starting sync with peer");
                    peer.state = PeerState::Syncing { their_heads };
                }
            }
            (SyncMessage::Sync { data }, PeerState::Unavailable | PeerState::RequestedFromUs) => {
                // Weird, they said this wasn't available, but they have it now so oh well
                let sync_msg = match sync::Message::decode(&data) {
                    Ok(msg) => msg,
                    Err(e) => {
                        tracing::warn!(
                            connection_id=?conn.connection_id, err=?e,
                            "failed to decode sync message, marking peer as unavailable"
                        );
                        peer.state = PeerState::Unavailable;
                        return;
                    }
                };
                if let Err(e) = conn.receive_sync_message(now, doc, sync_msg) {
                    tracing::warn!(
                        connection_id=?conn.connection_id, err=?e,
                        "failed to apply sync message, marking peer as unavailable"
                    );
                    peer.state = PeerState::Unavailable;
                    return;
                }

                let their_heads = conn.their_heads().unwrap_or_default();
                peer.state = PeerState::Syncing { their_heads };
            }
            (SyncMessage::Sync { data }, PeerState::Syncing { .. }) => {
                // They sent us a sync message while we were syncing, so we can just
                // apply it to our existing state
                let sync_msg = match sync::Message::decode(&data) {
                    Ok(msg) => msg,
                    Err(e) => {
                        tracing::warn!(
                            connection_id=?conn.connection_id, err=?e,
                            "failed to decode sync message, marking peer as unavailable"
                        );
                        peer.state = PeerState::Unavailable;
                        return;
                    }
                };
                if let Err(e) = conn.receive_sync_message(now, doc, sync_msg) {
                    tracing::warn!(
                        connection_id=?conn.connection_id, err=?e,
                        "failed to apply sync message, marking peer as unavailable"
                    );
                    peer.state = PeerState::Unavailable;
                }
            }
            (
                SyncMessage::DocUnavailable,
                PeerState::Requesting { .. } | PeerState::RequestedFromUs,
            ) => {
                peer.state = PeerState::Unavailable;
            }
            (SyncMessage::DocUnavailable, PeerState::Unavailable) => {
                // Nothing to do, they're already unavailable
            }
            (SyncMessage::DocUnavailable, PeerState::Syncing { .. }) => {
                // weird, they must have lost the doc somehow. Oh well
                peer.state = PeerState::Unavailable;
            }
        }
    }

    pub(crate) fn generate_message(
        &mut self,
        now: UnixTimestamp,
        doc: &Automerge,
        conn: &mut PeerDocConnection,
    ) -> Option<SyncMessage> {
        let any_peer_is_syncing = self
            .peer_states
            .values()
            .any(|s| matches!(s.state, PeerState::Syncing { .. }));
        let Some(peer) = self.peer_states.get_mut(&conn.connection_id) else {
            tracing::warn!(conn_id=?conn.connection_id, "no peer state for connection ID");
            return None;
        };
        match &mut peer.state {
            PeerState::Requesting(requesting) => {
                if !matches!(requesting, Requesting::AwaitingSend) {
                    return None;
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
                    return None;
                }
                conn.reset_sync_state();
                *requesting = Requesting::Sent;
                conn.generate_sync_message(now, doc)
                    .map(|msg| SyncMessage::Request { data: msg.encode() })
            }
            PeerState::Syncing { .. } => conn
                .generate_sync_message(now, doc)
                .map(|msg| SyncMessage::Sync { data: msg.encode() }),
            PeerState::Unavailable | PeerState::RequestedFromUs => None,
        }
    }

    pub(crate) fn status(&self, doc: &Automerge, any_dialer_connecting: bool) -> RequestState {
        if tracing::enabled!(tracing::Level::TRACE) {
            tracing::trace!(?self.peer_states, any_dialer_connecting, "checking if request is done");
        }
        let all_peers_unavailable = self.peer_states.values().all(|peer| {
            matches!(
                peer.state,
                PeerState::Unavailable
                    | PeerState::RequestedFromUs
                    | PeerState::Requesting(Requesting::NotSentDueToAnnouncePolicy)
            )
        });

        // If a dialer is connecting we expect a new peer soon, so don't
        // consider all avenues exhausted even if every current peer is
        // unavailable.
        let all_unavailable = all_peers_unavailable && !any_dialer_connecting;

        if all_peers_unavailable && any_dialer_connecting {
            tracing::debug!(
                "All current peers are unavailable but a dialer is connecting, waiting"
            );
        } else if all_unavailable {
            tracing::debug!("All peers are unavailable, sync complete");
        }

        let any_sync_is_done = self.peer_states.values().any(|peer| {
            matches!(&peer.state, PeerState::Syncing { their_heads } if their_heads.iter().all(|head| doc.get_change_by_hash(head).is_some()))
        });
        if any_sync_is_done {
            tracing::debug!("At least one peer has completed syncing, sync complete");
        }

        tracing::trace!(
            ?all_unavailable,
            ?any_sync_is_done,
            any_dialer_connecting,
            "request status check"
        );

        RequestState {
            finished: all_unavailable || any_sync_is_done,
            found: (!all_unavailable) && any_sync_is_done,
        }
    }

    pub(crate) fn peers_waiting_for_us_to_respond(&self) -> impl Iterator<Item = ConnectionId> {
        self.peer_states
            .iter()
            .filter_map(|(conn_id, peer)| match peer.state {
                PeerState::RequestedFromUs => Some(*conn_id),
                _ => None,
            })
    }

    pub(crate) fn announce_policy_changed(&mut self, peer: ConnectionId, policy: AnnouncePolicy) {
        let Some(peer) = self.peer_states.get_mut(&peer) else {
            return;
        };
        if let PeerState::Requesting(requesting) = &peer.state {
            match requesting {
                Requesting::AwaitingAnnouncePolicy => match policy {
                    AnnouncePolicy::Announce => {
                        peer.state = PeerState::Requesting(Requesting::AwaitingSend);
                    }
                    AnnouncePolicy::DontAnnounce => {
                        peer.state = PeerState::Requesting(Requesting::NotSentDueToAnnouncePolicy);
                    }
                    _ => {}
                },
                Requesting::NotSentDueToAnnouncePolicy => {
                    if policy == AnnouncePolicy::Announce {
                        peer.state = PeerState::Requesting(Requesting::AwaitingSend);
                    }
                }
                _ => {}
            }
        }
    }
}
