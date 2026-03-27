use std::collections::HashMap;

use crate::{
    DialerId, UnixTimestamp,
    actors::{
        document::io::DocumentIoResult,
        messages::{DocDialerState, DocMessage, HubToDocMsgPayload},
    },
    io::IoResult,
};

/// Internal input messages for the async actor runtime.
///
/// These messages are sent from the synchronous `handle_message` interface
/// to the internal async `actor_run` function.
#[derive(Debug)]
pub(crate) enum ActorInput {
    Terminate,
    /// Handle incoming sync message
    HandleDocMessage {
        connection_id: crate::ConnectionId,
        message: DocMessage,
        received_at: UnixTimestamp,
    },
    NewConnection {
        connection_id: crate::ConnectionId,
        peer_id: crate::PeerId,
    },
    ConnectionClosed {
        connection_id: crate::ConnectionId,
    },
    IoComplete(IoResult<DocumentIoResult>),
    Request,
    Tick,
    /// The set of dialer states has changed.
    DialerStatesChanged {
        dialers: HashMap<DialerId, DocDialerState>,
    },
    /// Apply data received via subduction to the automerge document.
    #[cfg(feature = "subduction")]
    ApplySubductionData {
        blobs: Vec<Vec<u8>>,
    },
    /// Update on subduction's search for this document.
    #[cfg(feature = "subduction")]
    SubductionRequestStatus {
        status: crate::actors::messages::SubductionSearchStatus,
    },
}

/// Convert external messages to internal input messages
impl From<HubToDocMsgPayload> for ActorInput {
    fn from(message: HubToDocMsgPayload) -> Self {
        match message {
            HubToDocMsgPayload::Terminate => ActorInput::Terminate,
            HubToDocMsgPayload::HandleDocMessage {
                connection_id,
                message,
                received_at,
            } => ActorInput::HandleDocMessage {
                connection_id,
                message,
                received_at,
            },
            HubToDocMsgPayload::NewConnection {
                connection_id,
                peer_id,
            } => ActorInput::NewConnection {
                connection_id,
                peer_id,
            },
            HubToDocMsgPayload::ConnectionClosed { connection_id } => {
                ActorInput::ConnectionClosed { connection_id }
            }
            HubToDocMsgPayload::RequestAgain => ActorInput::Request,
            HubToDocMsgPayload::DialerStatesChanged { dialers } => {
                ActorInput::DialerStatesChanged { dialers }
            }
            #[cfg(feature = "subduction")]
            HubToDocMsgPayload::ApplySubductionData { blobs } => {
                ActorInput::ApplySubductionData { blobs }
            }
            #[cfg(feature = "subduction")]
            HubToDocMsgPayload::SubductionRequestStatus { status } => {
                ActorInput::SubductionRequestStatus { status }
            }
        }
    }
}
