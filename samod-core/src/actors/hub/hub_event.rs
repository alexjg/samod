use automerge::{Automerge, transaction::CommitOptions};

use crate::{
    ConnectionId, DialerId, DocumentActorId, DocumentId, ListenerId,
    actors::{
        DocToHubMsg,
        hub::{Command, CommandId},
    },
    io::IoResult,
    network::{DialerConfig, ListenerConfig},
};

use super::{DispatchedCommand, HubEventPayload, HubInput, io::HubIoResult};

/// An event that can be processed by the hub actor
///
/// Events are the primary mechanism for interacting with the hub. They represent
/// either commands to be executed or notifications that some external operation
/// has completed.
///
/// Events are created using the static methods on this struct, which provide
/// type-safe construction of different event types.
#[derive(Debug, Clone)]
pub struct HubEvent {
    pub(crate) payload: HubEventPayload,
}

impl HubEvent {
    /// Creates an event indicating that an IO operation has completed.
    pub fn io_complete(result: IoResult<HubIoResult>) -> Self {
        HubEvent {
            payload: HubEventPayload::IoComplete(result),
        }
    }

    /// Creates a tick event for periodic processing.
    pub fn tick() -> Self {
        HubEvent {
            payload: HubEventPayload::Input(HubInput::Tick),
        }
    }

    /// Creates an event indicating that a message was received from a document actor.
    pub fn actor_message(actor_id: DocumentActorId, message: DocToHubMsg) -> Self {
        HubEvent {
            payload: HubEventPayload::Input(HubInput::ActorMessage {
                actor_id,
                message: message.0,
            }),
        }
    }

    /// Creates a command to receive a message on a specific connection.
    pub fn receive(connection_id: ConnectionId, msg: Vec<u8>) -> DispatchedCommand {
        Self::dispatch_command(Command::Receive { connection_id, msg })
    }

    /// Creates an event indicating that a document actor is ready.
    pub fn actor_ready(document_id: DocumentId) -> DispatchedCommand {
        Self::dispatch_command(Command::ActorReady { document_id })
    }

    /// Creates a command to create a new document.
    pub fn create_document(mut initial_content: Automerge) -> DispatchedCommand {
        if initial_content.is_empty() {
            initial_content.empty_commit(CommitOptions::default());
        }
        Self::dispatch_command(Command::CreateDocument {
            content: Box::new(initial_content),
        })
    }

    /// Creates a command to find and load an existing document.
    pub fn find_document(document_id: DocumentId) -> DispatchedCommand {
        Self::dispatch_command(Command::FindDocument { document_id })
    }

    /// Creates an event indicating that a network connection has been lost externally.
    pub fn connection_lost(connection_id: ConnectionId) -> Self {
        HubEvent {
            payload: HubEventPayload::Input(HubInput::ConnectionLost { connection_id }),
        }
    }

    /// Register a new dialer.
    ///
    /// The first `DialRequest` is emitted immediately in the returned `HubResults`.
    pub fn add_dialer(config: DialerConfig) -> DispatchedCommand {
        let command_id = CommandId::new();
        DispatchedCommand {
            command_id,
            event: HubEvent {
                payload: HubEventPayload::Input(HubInput::AddDialer { command_id, config }),
            },
        }
    }

    /// Register a new listener.
    ///
    /// No `DialRequest` is emitted — the IO layer is responsible for
    /// accepting inbound transports and calling `create_listener_connection`
    /// for each one.
    pub fn add_listener(config: ListenerConfig) -> DispatchedCommand {
        let command_id = CommandId::new();
        DispatchedCommand {
            command_id,
            event: HubEvent {
                payload: HubEventPayload::Input(HubInput::AddListener { command_id, config }),
            },
        }
    }

    /// Create a connection for a dialer.
    ///
    /// Called by the IO layer after successfully establishing a transport.
    /// The connection is immediately associated with the dialer.
    /// The dialer transitions from `TransportPending` to `Connected`.
    pub fn create_dialer_connection(dialer_id: DialerId) -> DispatchedCommand {
        let command_id = CommandId::new();
        DispatchedCommand {
            command_id,
            event: HubEvent {
                payload: HubEventPayload::Input(HubInput::CreateDialerConnection {
                    command_id,
                    dialer_id,
                }),
            },
        }
    }

    /// Create a connection for a listener.
    ///
    /// Called by the IO layer after accepting an inbound transport.
    /// The connection is immediately added to the listener's active set.
    pub fn create_listener_connection(listener_id: ListenerId) -> DispatchedCommand {
        let command_id = CommandId::new();
        DispatchedCommand {
            command_id,
            event: HubEvent {
                payload: HubEventPayload::Input(HubInput::CreateListenerConnection {
                    command_id,
                    listener_id,
                }),
            },
        }
    }

    /// The IO layer failed to establish a transport for a dialer.
    ///
    /// Triggers backoff and schedules a retry, or transitions to `Failed`
    /// if max retries have been exceeded.
    pub fn dial_failed(dialer_id: DialerId, error: String) -> HubEvent {
        HubEvent {
            payload: HubEventPayload::Input(HubInput::DialFailed { dialer_id, error }),
        }
    }

    /// Remove a dialer. The active connection (if any) is closed. No further
    /// dial requests will be emitted.
    pub fn remove_dialer(dialer_id: DialerId) -> HubEvent {
        HubEvent {
            payload: HubEventPayload::Input(HubInput::RemoveDialer { dialer_id }),
        }
    }

    /// Remove a listener. All active connections are closed.
    pub fn remove_listener(listener_id: ListenerId) -> HubEvent {
        HubEvent {
            payload: HubEventPayload::Input(HubInput::RemoveListener { listener_id }),
        }
    }

    pub fn stop() -> HubEvent {
        HubEvent {
            payload: HubEventPayload::Input(HubInput::Stop),
        }
    }

    /// Internal helper to create a dispatched command with a unique ID.
    fn dispatch_command(command: Command) -> DispatchedCommand {
        let command_id = CommandId::new();
        DispatchedCommand {
            command_id,
            event: HubEvent {
                payload: HubEventPayload::Input(HubInput::Command {
                    command_id,
                    command: Box::new(command),
                }),
            },
        }
    }
}

impl std::fmt::Display for HubEvent {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:?}", self.payload)
    }
}
