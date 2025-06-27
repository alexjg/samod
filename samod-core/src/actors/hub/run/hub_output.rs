use crate::{
    DocumentActorId,
    actors::{
        document::SpawnArgs,
        hub::{CommandId, CommandResult},
        messages::HubToDocMsgPayload,
    },
    network::ConnectionEvent,
};

#[derive(Debug)]
pub(crate) enum HubOutput {
    CommandCompleted {
        command_id: CommandId,
        result: CommandResult,
    },
    SpawnActor {
        args: Box<SpawnArgs>,
    },
    SendToActor {
        actor_id: DocumentActorId,
        message: HubToDocMsgPayload,
    },
    ConnectionEvent {
        event: ConnectionEvent,
    },
}
