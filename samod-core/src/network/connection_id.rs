use minicbor::{Decode, Encode};
use std::sync::atomic::{AtomicU32, Ordering};

static LAST_CONNECTION_ID: AtomicU32 = AtomicU32::new(0);

/// A unique identifier for network connections in the samod-core system.
///
/// `ConnectionId` represents a communication channel that can be used for sending
/// and receiving messages. Each connection has a unique identifier that is automatically
/// generated when the connection is created.
///
/// ## Usage
///
/// Connections are created through the `CreateConnection` command and are used to scope
/// network operations. All send and receive operations specify which connection
/// they operate on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ConnectionId(u32);

impl ConnectionId {
    pub(crate) fn new() -> Self {
        let id = LAST_CONNECTION_ID.fetch_add(1, Ordering::SeqCst);
        ConnectionId(id)
    }
}

impl From<ConnectionId> for u32 {
    fn from(id: ConnectionId) -> Self {
        id.0
    }
}

impl From<u32> for ConnectionId {
    fn from(id: u32) -> Self {
        ConnectionId(id)
    }
}
