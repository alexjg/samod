use std::collections::HashMap;

use crate::{
    ConnectionId, DocumentId, PeerId, UnixTimestamp,
    actors::{hub::HubResults, messages::SyncMessage},
    network::{
        ConnDirection, ConnectionInfo, ConnectionOwner, ConnectionState, PeerDocState,
        PeerMetadata, wire_protocol::WireMessage,
    },
};

use super::{EstablishedConnection, ReceiveEvent};

/// State of a network connection throughout its lifecycle.
#[derive(Debug, Clone)]
pub struct Connection {
    id: ConnectionId,
    /// The dialer or listener that owns this connection.
    owner: ConnectionOwner,
    local_peer_id: PeerId,
    local_metadata: Option<PeerMetadata>,
    /// Current phase of the connection
    phase: ConnectionPhase,
    /// When the connection was created
    #[allow(dead_code)]
    created_at: UnixTimestamp,
    last_received: Option<UnixTimestamp>,
    last_sent: Option<UnixTimestamp>,
    // Whether our state has changed since we last notified the event loop
    // (via `pop_new_state`)
    dirty: bool,
}

/// The phase of a connection's lifecycle.
#[derive(Debug, Clone)]
pub(crate) enum ConnectionPhase {
    WaitingForPeer,
    WaitingForJoin,
    Established(EstablishedConnection),
    Closed,
}

pub(crate) struct ConnectionArgs {
    pub(crate) direction: ConnDirection,
    pub(crate) owner: ConnectionOwner,
    pub(crate) local_peer_id: PeerId,
    pub(crate) local_metadata: Option<PeerMetadata>,
    pub(crate) created_at: UnixTimestamp,
}

impl Connection {
    /// Create a new connection in handshaking state.
    pub(crate) fn new_handshaking(
        out: &mut HubResults,
        ConnectionArgs {
            direction,
            owner,
            local_peer_id,
            local_metadata,
            created_at,
        }: ConnectionArgs,
    ) -> Self {
        let mut conn = Self {
            id: ConnectionId::new(),
            owner,
            local_peer_id: local_peer_id.clone(),
            local_metadata: local_metadata.clone(),
            phase: ConnectionPhase::WaitingForJoin,
            created_at,
            last_received: None,
            last_sent: None,
            dirty: true, // We want to immediately notify of our new state
        };
        if let ConnDirection::Outgoing = direction {
            conn.phase = ConnectionPhase::WaitingForPeer;
            tracing::trace!(conn_id=?conn.id, "sending join message");
            conn.send(
                out,
                created_at,
                WireMessage::Join {
                    sender_id: local_peer_id.clone(),
                    supported_protocol_versions: vec!["1".to_string()],
                    metadata: local_metadata.as_ref().map(|meta| meta.to_wire()),
                },
            );
        }
        conn
    }

    pub(crate) fn id(&self) -> ConnectionId {
        self.id
    }

    pub(crate) fn owner(&self) -> ConnectionOwner {
        self.owner
    }

    pub(crate) fn receive_msg(
        &mut self,
        out: &mut HubResults,
        now: UnixTimestamp,
        msg: WireMessage,
    ) -> Vec<ReceiveEvent> {
        self.dirty = true;
        self.last_received = Some(now);
        match self.phase {
            ConnectionPhase::WaitingForJoin => match msg {
                WireMessage::Join {
                    sender_id,
                    supported_protocol_versions,
                    metadata,
                } => {
                    tracing::trace!(
                        conn_id=?self.id,
                        ?sender_id,
                        ?supported_protocol_versions,
                        "received Join message from peer"
                    );
                    if !supported_protocol_versions.contains(&"1".to_string()) {
                        tracing::warn!(conn_id=?self.id, "peer does not support protocol version 1");
                        self.send(
                            out,
                            now,
                            WireMessage::Error {
                                message: "unsupported protocol version".to_string(),
                            },
                        );
                        self.phase = ConnectionPhase::Closed;
                        return Vec::new();
                    }
                    tracing::trace!(conn_id=?self.id, "sending Peer message in response to Join");
                    self.send(
                        out,
                        now,
                        WireMessage::Peer {
                            sender_id: self.local_peer_id.clone(),
                            selected_protocol_version: "1".to_string(),
                            target_id: sender_id.clone(),
                            metadata: self.local_metadata.as_ref().map(|meta| meta.to_wire()),
                        },
                    );
                    self.phase = ConnectionPhase::Established(EstablishedConnection {
                        remote_peer_id: sender_id.clone(),
                        remote_metadata: metadata.map(PeerMetadata::from_wire),
                        protocol_version: "1".to_string(),
                        established_at: now,
                        document_subscriptions: HashMap::new(),
                    });
                    vec![ReceiveEvent::HandshakeComplete {
                        remote_peer_id: sender_id,
                    }]
                }
                other => {
                    tracing::warn!(
                        message=?other,
                        conn_id=?self.id,
                        "unexpected message received in WaitingForJoin phase"
                    );
                    self.send(
                        out,
                        now,
                        WireMessage::Error {
                            message: "expected a join message".to_string(),
                        },
                    );
                    self.phase = ConnectionPhase::Closed;
                    Vec::new()
                }
            },
            ConnectionPhase::WaitingForPeer => match msg {
                WireMessage::Peer {
                    sender_id,
                    selected_protocol_version,
                    target_id,
                    metadata,
                } => {
                    tracing::trace!(
                        conn_id=?self.id,
                        ?sender_id,
                        ?selected_protocol_version,
                        ?target_id,
                        "received Peer message from peer"
                    );
                    if selected_protocol_version != "1" {
                        tracing::warn!(conn_id=?self.id, "peer does not support protocol version 1");
                        self.send(
                            out,
                            now,
                            WireMessage::Error {
                                message: "unsupported protocol version".to_string(),
                            },
                        );
                        self.phase = ConnectionPhase::Closed;
                        return Vec::new();
                    }
                    self.phase = ConnectionPhase::Established(EstablishedConnection {
                        remote_peer_id: sender_id.clone(),
                        remote_metadata: metadata.map(PeerMetadata::from_wire),
                        protocol_version: selected_protocol_version,
                        established_at: now,
                        document_subscriptions: HashMap::new(),
                    });
                    vec![ReceiveEvent::HandshakeComplete {
                        remote_peer_id: sender_id,
                    }]
                }
                other => {
                    tracing::warn!(
                        message=?other,
                        conn_id=?self.id,
                        "unexpected message received in WaitingForPeer phase"
                    );
                    self.send(
                        out,
                        now,
                        WireMessage::Error {
                            message: "expected a peer message".to_string(),
                        },
                    );
                    self.phase = ConnectionPhase::Closed;
                    Vec::new()
                }
            },
            ConnectionPhase::Established(_) => match msg {
                WireMessage::Join { .. } | WireMessage::Peer { .. } => {
                    tracing::warn!(
                        message=?msg,
                        conn_id=?self.id,
                        "unexpected Join or Peer message received in Established phase"
                    );
                    self.send(
                        out,
                        now,
                        WireMessage::Error {
                            message: "unexpected join or peer message".to_string(),
                        },
                    );
                    self.phase = ConnectionPhase::Closed;
                    Vec::new()
                }
                WireMessage::Leave { sender_id } => {
                    tracing::trace!(conn_id=?self.id, ?sender_id, "received Leave message");
                    self.phase = ConnectionPhase::Closed;
                    Vec::new()
                }
                WireMessage::Request {
                    document_id,
                    sender_id,
                    target_id,
                    data,
                } => vec![ReceiveEvent::SyncMessage {
                    doc_id: document_id,
                    sender_id,
                    target_id,
                    msg: SyncMessage::Request { data },
                }],
                WireMessage::Sync {
                    document_id,
                    sender_id,
                    target_id,
                    data,
                } => vec![ReceiveEvent::SyncMessage {
                    doc_id: document_id,
                    sender_id,
                    target_id,
                    msg: SyncMessage::Sync { data },
                }],
                WireMessage::DocUnavailable {
                    sender_id,
                    target_id,
                    document_id,
                } => vec![ReceiveEvent::SyncMessage {
                    doc_id: document_id,
                    sender_id,
                    target_id,
                    msg: SyncMessage::DocUnavailable,
                }],
                WireMessage::Ephemeral {
                    sender_id,
                    target_id,
                    count,
                    session_id,
                    document_id,
                    data,
                } => {
                    vec![ReceiveEvent::EphemeralMessage {
                        doc_id: document_id,
                        sender_id,
                        target_id,
                        count,
                        session_id: session_id.into(),
                        msg: data,
                    }]
                }
                WireMessage::RemoteHeadsChanged { .. }
                | WireMessage::RemoteSubscriptionChange { .. } => vec![],
                WireMessage::Error { message } => {
                    tracing::warn!(
                        conn_id=?self.id,
                        "received error message in established phase: {}",
                        message
                    );
                    self.phase = ConnectionPhase::Closed;
                    Vec::new()
                }
            },
            ConnectionPhase::Closed => {
                tracing::warn!(conn_id=?self.id, "received message in closed connection phase");
                Vec::new()
            }
        }
    }

    /// Get the established connection if in established phase.
    pub(crate) fn established_connection(&self) -> Option<&EstablishedConnection> {
        match &self.phase {
            ConnectionPhase::Established(conn) => Some(conn),
            _ => None,
        }
    }

    /// Get the established connection if in established phase.
    pub(crate) fn established_connection_mut(&mut self) -> Option<&mut EstablishedConnection> {
        match &mut self.phase {
            ConnectionPhase::Established(conn) => Some(conn),
            _ => None,
        }
    }

    /// Get the remote peer ID if established.
    pub(crate) fn remote_peer_id(&self) -> Option<&PeerId> {
        if let ConnectionPhase::Established(EstablishedConnection { remote_peer_id, .. }) =
            &self.phase
        {
            Some(remote_peer_id)
        } else {
            None
        }
    }

    /// Add a document subscription.
    pub(crate) fn add_document(&mut self, document_id: DocumentId) {
        let ConnectionPhase::Established(established) = &mut self.phase else {
            panic!("Cannot add document subscription in non-established phase");
        };
        established
            .document_subscriptions
            .insert(document_id.clone(), PeerDocState::empty());
    }

    pub(crate) fn update_peer_state(&mut self, document_id: &DocumentId, state: PeerDocState) {
        let ConnectionPhase::Established(established) = &mut self.phase else {
            tracing::warn!("attmpeted to update document for non-established connection");
            return;
        };
        if let Some(doc_state) = established.document_subscriptions.get_mut(document_id) {
            self.dirty = true;
            *doc_state = state;
        } else {
            tracing::warn!(?document_id, "tried to update state for unknown document",);
        }
    }

    pub(crate) fn is_closed(&self) -> bool {
        matches!(self.phase, ConnectionPhase::Closed)
    }

    fn send(&mut self, out: &mut HubResults, now: UnixTimestamp, msg: WireMessage) {
        self.dirty = true;
        self.last_sent = Some(now);
        out.send(self, msg.encode());
    }

    pub(crate) fn last_received(&self) -> Option<UnixTimestamp> {
        self.last_received
    }

    pub(crate) fn last_sent(&self) -> Option<UnixTimestamp> {
        self.last_sent
    }

    pub(crate) fn info(&self) -> ConnectionInfo {
        let (doc_connections, state) = match &self.phase {
            ConnectionPhase::Established(established) => (
                established.document_subscriptions().clone(),
                ConnectionState::Connected {
                    their_peer_id: established.remote_peer_id().clone(),
                },
            ),
            _ => (HashMap::new(), ConnectionState::Handshaking),
        };
        ConnectionInfo {
            id: self.id,
            last_received: self.last_received,
            last_sent: self.last_sent,
            docs: doc_connections,
            state,
        }
    }

    // If this connection state has changed since the last call to
    // `pop_new_info`, return the new info
    pub(crate) fn pop_new_info(&mut self) -> Option<ConnectionInfo> {
        if self.dirty {
            self.dirty = false;
            Some(self.info())
        } else {
            None
        }
    }
}
