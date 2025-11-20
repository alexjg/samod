use crate::{
    ConnectionId, DocumentActorId,
    actors::{
        hub::{Command, CommandId},
        messages::DocToHubMsgPayload,
    },
};

#[derive(Debug, Clone)]
pub(crate) enum HubInput {
    Command {
        command_id: CommandId,
        command: Box<Command>,
    },
    Tick,
    /// Message received from a document actor
    ActorMessage {
        actor_id: DocumentActorId,
        message: DocToHubMsgPayload,
    },
    /// Notification that a network connection has been lost externally
    ConnectionLost {
        connection_id: ConnectionId,
    },
    Stop,
}
