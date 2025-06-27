/// Indicates whether a connection was initiated locally or received from a remote peer
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConnDirection {
    /// Connection initiated by this peer (we are the "initiating peer")
    Outgoing,
    /// Connection accepted from a remote peer (we are the "receiving peer")  
    Incoming,
}
