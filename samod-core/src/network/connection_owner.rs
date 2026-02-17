use super::{DialerId, ListenerId};

/// Identifies which dialer or listener owns a connection.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ConnectionOwner {
    /// The connection belongs to a dialer (outgoing).
    Dialer(DialerId),
    /// The connection belongs to a listener (incoming).
    Listener(ListenerId),
}
