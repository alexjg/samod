use std::collections::HashMap;

use crate::{ConnectionId, DialerId};

use super::DocMessage;

/// Dialer state as visible to document actors.
///
/// This is a simplified projection of the hub-internal `DialerStatus` that
/// strips out hub internals (connection IDs, retry timestamps) and only
/// exposes what document actors need to make availability decisions.
///
/// One significant difference to the `DialerStatus` is that we only report
/// `Connected` if the connection is fully established (i.e. the connection
/// exists and has a remote peer ID). This is because document actors only care
/// about whether the connection is actually usable, and reporting `Connected`
/// too early (e.g. while still in the handshake phase) could mean we mark a
/// document as unavailable when we are just waiting for a handshake to
/// complete
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum DocDialerState {
    /// Transport is being established (NeedTransport or TransportPending).
    Connecting,
    /// An active connection exists.
    Connected,
    /// Waiting for backoff before retrying.
    WaitingToRetry,
    /// Permanently failed (max retries exceeded).
    Failed,
}

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

    /// Notify the actor of the current dialer states.
    ///
    /// Sent whenever the set of dialer states changes. The full map is sent
    /// each time (not deltas) for robustness.
    DialerStatesChanged {
        dialers: HashMap<DialerId, DocDialerState>,
    },
}
