use crate::ConnectionId;

use super::{ConnectionOwner, PeerInfo, connection_info::ConnectionInfo};

/// Events related to connection lifecycle and handshake process.
///
/// These events are emitted during connection establishment, handshake
/// completion, and connection failures. They allow applications to track
/// the state of network connections and respond to connectivity changes.
///
/// Each event includes a `ConnectionOwner` that identifies the dialer
/// or listener that owns the connection.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConnectionEvent {
    /// Handshake completed successfully with a peer.
    ///
    /// This event is emitted when the connection handshake process
    /// finishes successfully and the connection transitions to the
    /// established state. After this event, the connection is ready
    /// for document synchronization.
    HandshakeCompleted {
        connection_id: ConnectionId,
        owner: ConnectionOwner,
        peer_info: PeerInfo,
    },
    /// Connection failed or was disconnected.
    ///
    /// This event is emitted when a connection fails or when a connection is
    /// explicitly disconnected. This can happen due to network errors, protocol
    /// violations, or explicit disconnection.
    ConnectionFailed {
        connection_id: ConnectionId,
        owner: ConnectionOwner,
        error: String,
    },

    /// This event is emitted whenever some part of the connection state changes
    StateChanged {
        connection_id: ConnectionId,
        owner: ConnectionOwner,
        // The new state
        new_state: ConnectionInfo,
    },
}

impl ConnectionEvent {
    /// Get the connection ID associated with this event.
    pub fn connection_id(&self) -> ConnectionId {
        match self {
            ConnectionEvent::HandshakeCompleted { connection_id, .. } => *connection_id,
            ConnectionEvent::ConnectionFailed { connection_id, .. } => *connection_id,
            ConnectionEvent::StateChanged { connection_id, .. } => *connection_id,
        }
    }

    /// Get the owner of the connection associated with this event.
    pub fn owner(&self) -> ConnectionOwner {
        match self {
            ConnectionEvent::HandshakeCompleted { owner, .. } => *owner,
            ConnectionEvent::ConnectionFailed { owner, .. } => *owner,
            ConnectionEvent::StateChanged { owner, .. } => *owner,
        }
    }
}
