use std::collections::HashMap;

use crate::{ConnectionId, DocumentId, PeerId, UnixTimestamp};

/// Information about each live connection
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConnectionInfo {
    pub id: ConnectionId,
    pub last_received: Option<UnixTimestamp>,
    pub last_sent: Option<UnixTimestamp>,
    /// The state of each document we are synchronizing with this peer
    pub docs: HashMap<DocumentId, PeerDocState>,
    /// Whether we are handshaking or connected with this peer
    pub state: ConnectionState,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConnectionState {
    /// We're still exchanging peer IDs
    Handshaking,
    /// We have exchanged peer IDs and we're now synchronizing documents
    Connected { their_peer_id: PeerId },
}

/// The state of synchronization for one (peer, document) pair
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd)]
pub struct PeerDocState {
    /// When we last received a message from this peer
    pub last_received: Option<UnixTimestamp>,
    /// When we last sent a message to this peer
    pub last_sent: Option<UnixTimestamp>,
    /// The heads of the document when we last sent a message
    pub last_sent_heads: Option<Vec<automerge::ChangeHash>>,
    /// The last heads of the document that the peer said they had
    pub last_acked_heads: Option<Vec<automerge::ChangeHash>>,
}

impl PeerDocState {
    pub(crate) fn empty() -> Self {
        Self {
            last_received: None,
            last_sent: None,
            last_sent_heads: None,
            last_acked_heads: None,
        }
    }
}
