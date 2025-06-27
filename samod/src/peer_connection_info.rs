use std::collections::HashMap;

use samod_core::{DocumentId, PeerId};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConnectionInfo {
    pub last_received: Option<chrono::DateTime<chrono::Utc>>,
    pub last_sent: Option<chrono::DateTime<chrono::Utc>>,
    pub docs: HashMap<DocumentId, PeerDocState>,
    pub state: ConnectionState,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConnectionState {
    Handshaking,
    Connected { their_peer_id: PeerId },
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
}
