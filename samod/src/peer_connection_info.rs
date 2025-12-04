use std::collections::HashMap;

use chrono::DateTime;
use samod_core::{ConnectionId, DocumentId, PeerId};

/// The state of a connection to one peer
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConnectionInfo {
    /// The ID of this connection
    pub id: ConnectionId,
    /// Last time we received a message from this peer
    pub last_received: Option<chrono::DateTime<chrono::Utc>>,
    /// Last time we sent a message to this peer
    pub last_sent: Option<chrono::DateTime<chrono::Utc>>,
    /// The state of each docuement we are synchronizing with this peer
    pub docs: HashMap<DocumentId, PeerDocState>,
    /// Whether we are handshaking or connected with this peer
    pub state: ConnectionState,
}

impl From<samod_core::network::ConnectionInfo> for ConnectionInfo {
    fn from(value: samod_core::network::ConnectionInfo) -> Self {
        ConnectionInfo {
            id: value.id,
            last_received: value
                .last_received
                .map(|i| DateTime::from_timestamp_millis(i.as_millis() as i64).unwrap()),
            last_sent: value
                .last_sent
                .map(|i| DateTime::from_timestamp_millis(i.as_millis() as i64).unwrap()),
            docs: value.docs.into_iter().map(|(k, v)| (k, v.into())).collect(),
            state: value.state.into(),
        }
    }
}

/// Whether we're still exchanging peer IDs or connected to a peer
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConnectionState {
    /// We're still exchanging peer IDs
    Handshaking,
    /// We have exchanged peer IDs and we're now synchronizing documents
    Connected { their_peer_id: PeerId },
}

impl From<samod_core::network::ConnectionState> for ConnectionState {
    fn from(value: samod_core::network::ConnectionState) -> Self {
        match value {
            samod_core::network::ConnectionState::Handshaking => ConnectionState::Handshaking,
            samod_core::network::ConnectionState::Connected { their_peer_id } => {
                ConnectionState::Connected { their_peer_id }
            }
        }
    }
}

/// The state of sycnhronization of a document with a remote peer obtained via [`RepoHandle::peer_doc_state`](crate::RepoHandle::peer_doc_state)
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd)]
pub struct PeerDocState {
    /// When we last received a message from this peer
    pub last_received: Option<chrono::DateTime<chrono::Utc>>,
    /// When we last sent a message to this peer
    pub last_sent: Option<chrono::DateTime<chrono::Utc>>,
    /// The heads of the document when we last sent a message
    pub last_sent_heads: Option<Vec<automerge::ChangeHash>>,
    /// The last heads of the document that the peer said they had
    pub last_acked_heads: Option<Vec<automerge::ChangeHash>>,
    /// The shared heads of the document that we have with the peer
    pub shared_heads: Option<Vec<automerge::ChangeHash>>,
    /// The last heads they said they had
    pub their_heads: Option<Vec<automerge::ChangeHash>>,
}

impl From<samod_core::network::PeerDocState> for PeerDocState {
    fn from(value: samod_core::network::PeerDocState) -> Self {
        PeerDocState {
            last_received: value
                .last_received
                .map(|t| DateTime::from_timestamp_millis(t.as_millis() as i64).unwrap()),
            last_sent: value
                .last_sent
                .map(|t| DateTime::from_timestamp_millis(t.as_millis() as i64).unwrap()),
            last_sent_heads: value.last_sent_heads,
            last_acked_heads: value.last_acked_heads,
            shared_heads: value.shared_heads,
            their_heads: value.their_heads,
        }
    }
}
