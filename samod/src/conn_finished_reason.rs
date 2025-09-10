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
    /// There was some error configuring a connection
    Error(String),
}
