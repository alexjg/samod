/// Why a connection future stopped
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConnFinishedReason {
    /// This repository is shutting down
    Shutdown,
    /// The other end disconnected gracefully
    TheyDisconnected,
    /// We are terminating the connection for some reason
    WeDisconnected,
    /// There was some error on the network transport when receiving data
    ErrorReceiving(String),
    /// There was some error on the network transport when sending data
    ErrorSending(String),
}

impl std::fmt::Display for ConnFinishedReason {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ConnFinishedReason::Shutdown => write!(f, "Repository shutting down"),
            ConnFinishedReason::WeDisconnected => write!(f, "We are disconnecting"),
            ConnFinishedReason::TheyDisconnected => write!(f, "They disconnected gracefully"),
            ConnFinishedReason::ErrorReceiving(msg) => write!(f, "Error receiving: {msg}"),
            ConnFinishedReason::ErrorSending(msg) => write!(f, "Error sending: {msg}"),
        }
    }
}
