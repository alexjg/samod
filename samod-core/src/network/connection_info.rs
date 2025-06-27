use std::collections::HashMap;

use crate::{ConnectionId, DocumentId, PeerId, UnixTimestamp};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConnectionInfo {
    pub id: ConnectionId,
    pub last_received: Option<UnixTimestamp>,
    pub last_sent: Option<UnixTimestamp>,
    pub docs: HashMap<DocumentId, PeerDocState>,
    pub state: ConnectionState,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConnectionState {
    Handshaking,
    Connected { their_peer_id: PeerId },
}

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
