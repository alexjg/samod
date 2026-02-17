use crate::{
    ConnectionId, DialerId, DocumentActorId, ListenerId,
    actors::{
        hub::{Command, CommandId},
        messages::DocToHubMsgPayload,
    },
    network::{DialerConfig, ListenerConfig},
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
    /// Register a new dialer
    AddDialer {
        command_id: CommandId,
        config: DialerConfig,
    },
    /// Register a new listener
    AddListener {
        command_id: CommandId,
        config: ListenerConfig,
    },
    /// Create a connection for a dialer (IO layer successfully established transport)
    CreateDialerConnection {
        command_id: CommandId,
        dialer_id: DialerId,
    },
    /// Create a connection for a listener (IO layer accepted an inbound transport)
    CreateListenerConnection {
        command_id: CommandId,
        listener_id: ListenerId,
    },
    /// The IO layer failed to establish a transport for a dialer
    DialFailed {
        dialer_id: DialerId,
        error: String,
    },
    /// Remove a dialer and close its connection
    RemoveDialer {
        dialer_id: DialerId,
    },
    /// Remove a listener and close all its connections
    RemoveListener {
        listener_id: ListenerId,
    },
    Stop,
}
