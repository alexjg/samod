use crate::ConnectionId;

use super::{PeerInfo, connection_info::ConnectionInfo};

/// Events related to connection lifecycle and handshake process.
///
/// These events are emitted during connection establishment, handshake
/// completion, and connection failures. They allow applications to track
/// the state of network connections and respond to connectivity changes.
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
        peer_info: PeerInfo,
    },
    /// Connection failed or was disconnected.
    ///
    /// This event is emitted when a connection fails or when a connection is
    /// explicitly disconnected. This can happen due to network errors, protocol
    /// violations, or explicit disconnection.
    ConnectionFailed {
        connection_id: ConnectionId,
        error: String,
    },

    /// This event is emitted whenever some part of the connection state changes
    StateChanged {
        connection_id: ConnectionId,
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
}
