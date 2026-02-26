use std::time::Duration;

use crate::ConnectionId;

/// Statistics about a single sync message processed by the document actor.
#[derive(Debug, Clone)]
pub struct SyncMessageStat {
    /// The connection this sync message was sent to or received from.
    pub connection_id: ConnectionId,
    /// Whether this message was received or generated.
    pub direction: SyncDirection,
    /// Size of the automerge sync data in bytes.
    pub bytes: usize,
    /// Wall-clock time spent on the automerge operation.
    pub duration: Duration,
}

/// Direction of a sync message.
#[derive(Debug, Clone, Copy)]
pub enum SyncDirection {
    /// A sync message received from a peer.
    Received,
    /// A sync message generated for a peer.
    Generated,
}
