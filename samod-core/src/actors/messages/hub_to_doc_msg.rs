use crate::ConnectionId;

use super::DocMessage;

/// Messages sent from the hub to document actors.
#[derive(Debug, Clone)]
pub struct HubToDocMsg(pub(crate) HubToDocMsgPayload);

#[derive(Debug, Clone)]
pub(crate) enum HubToDocMsgPayload {
    /// Request the actor to terminate gracefully.
    Terminate,

    NewConnection {
        connection_id: crate::ConnectionId,
        peer_id: crate::PeerId,
    },

    RequestAgain,

    /// Notify the actor that a connection has been closed.
    ConnectionClosed {
        connection_id: crate::ConnectionId,
    },

    HandleDocMessage {
        connection_id: ConnectionId,
        message: DocMessage,
    },
}
