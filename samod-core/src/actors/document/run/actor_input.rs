use crate::actors::messages::{DocMessage, HubToDocMsgPayload};

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
    },
    NewConnection {
        connection_id: crate::ConnectionId,
        peer_id: crate::PeerId,
    },
    ConnectionClosed {
        connection_id: crate::ConnectionId,
    },
    Request,
    Tick,
}

/// Convert external messages to internal input messages
impl From<HubToDocMsgPayload> for ActorInput {
    fn from(message: HubToDocMsgPayload) -> Self {
        match message {
            HubToDocMsgPayload::Terminate => ActorInput::Terminate,
            HubToDocMsgPayload::HandleDocMessage {
                connection_id,
                message,
            } => ActorInput::HandleDocMessage {
                connection_id,
                message,
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
        }
    }
}
