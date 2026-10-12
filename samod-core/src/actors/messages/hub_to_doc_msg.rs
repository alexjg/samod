use crate::{ConnectionId, UnixTimestamp};

use super::DocMessage;

/// Messages sent from the hub to document actors.
#[derive(Debug, Clone)]
pub struct HubToDocMsg(pub(crate) HubToDocMsgPayload);

impl HubToDocMsg {
    /// Install a sharing decision on all current connections to this peer,
    /// superseding any pending initial policy checks.
    pub fn update_share_policy(peer_id: crate::PeerId, config: crate::ShareConfig) -> Self {
        Self(HubToDocMsgPayload::UpdateSharePolicy { peer_id, config })
    }
}

#[derive(Debug, Clone)]
pub(crate) enum HubToDocMsgPayload {
    /// Request the actor to terminate gracefully.
    Terminate,

    UpdateSharePolicy {
        peer_id: crate::PeerId,
        config: crate::ShareConfig,
    },

    NewConnection {
        connection_id: crate::ConnectionId,
        peer_id: crate::PeerId,
    },

    /// Notify the actor that a connection has been closed.
    ConnectionClosed { connection_id: crate::ConnectionId },

    HandleDocMessage {
        connection_id: ConnectionId,
        message: DocMessage,
        received_at: UnixTimestamp,
    },
}
