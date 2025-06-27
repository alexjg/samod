use automerge::{Automerge, transaction::CommitOptions};

use crate::{
    ConnectionId, DocumentActorId, DocumentId,
    actors::{
        DocToHubMsg,
        hub::{Command, CommandId},
    },
    io::IoResult,
    network::ConnDirection,
};

use super::{DispatchedCommand, HubEventPayload, io::HubIoResult, run::HubInput};

/// An event that can be processed by the hub actor
///
/// Events are the primary mechanism for interacting with the hub. They represent
/// either commands to be executed or notifications that some external operation
/// has completed.
///
/// Events are created using the static methods on this struct, which provide
/// type-safe construction of different event types.
#[derive(Debug)]
pub struct HubEvent {
    pub(crate) payload: HubEventPayload,
}

impl HubEvent {
    /// Creates an event indicating that an IO operation has completed.
    ///
    /// This event is used to notify the hub that a previously requested
    /// IO operation (storage or network) has finished. The result contains
    /// the task ID and the outcome of the operation.
    ///
    /// # Arguments
    ///
    /// * `result` - The result of the completed IO operation
    pub fn io_complete(result: IoResult<HubIoResult>) -> Self {
        HubEvent {
            payload: HubEventPayload::IoComplete(result),
        }
    }

    /// Creates a tick event for periodic processing.
    ///
    /// Tick events are used to trigger periodic operations like cleanup,
    /// maintenance, or timeouts. They don't carry any data and are processed
    /// by the main event loop.
    pub fn tick() -> Self {
        HubEvent {
            payload: HubEventPayload::Input(HubInput::Tick),
        }
    }

    /// Creates an event indicating that a message was received from a document actor.
    ///
    /// This event is used when document actors (running in the caller's environment)
    /// send messages back to the hub.
    ///
    /// # Arguments
    ///
    /// * `actor_id` - The ID of the actor that sent the message
    /// * `message` - The message from the actor
    pub fn actor_message(actor_id: DocumentActorId, message: DocToHubMsg) -> Self {
        HubEvent {
            payload: HubEventPayload::Input(HubInput::ActorMessage {
                actor_id,
                message: message.0,
            }),
        }
    }

    /// Creates a command to receive a message on a specific connection.
    ///
    /// This represents an incoming message that should be processed by
    /// the system. The message is associated with a specific connection ID.
    ///
    /// # Arguments
    ///
    /// * `connection_id` - The ID of the connection on which the message was received
    /// * `msg` - The message content as bytes
    ///
    /// # Returns
    ///
    /// A `DispatchedCommand` containing both the command ID and the event
    /// to be processed. The command ID can be used to track completion.
    pub fn receive(connection_id: ConnectionId, msg: Vec<u8>) -> DispatchedCommand {
        Self::dispatch_command(Command::Receive { connection_id, msg })
    }

    /// Creates a command to create a new connection and begins handshake if outgoing.
    ///
    /// Connections are used for network communication. This command will
    /// create a new connection and return its ID when completed. For outgoing
    /// connections, the handshake will be initiated automatically.
    ///
    /// # Arguments
    ///
    /// * `direction` - Whether this is an outgoing or incoming connection
    ///
    /// # Returns
    ///
    /// A `DispatchedCommand` containing both the command ID and the event
    /// to be processed. The command ID can be used to retrieve the
    /// connection ID when the command completes.
    pub fn create_connection(direction: ConnDirection) -> DispatchedCommand {
        Self::dispatch_command(Command::CreateConnection { direction })
    }

    /// Creates an event indicating that a document actor is ready.
    pub fn actor_ready(document_id: DocumentId) -> DispatchedCommand {
        Self::dispatch_command(Command::ActorReady { document_id })
    }

    /// Creates a command to create a new document.
    ///
    /// # Returns
    ///
    /// A `DispatchedCommand` containing the command ID and event.
    /// The command ID can be used to track when the document creation completes.
    pub fn create_document(mut initial_content: Automerge) -> DispatchedCommand {
        if initial_content.is_empty() {
            initial_content.empty_commit(CommitOptions::default());
        }
        Self::dispatch_command(Command::CreateDocument {
            content: Box::new(initial_content),
        })
    }

    /// Creates a command to find and load an existing document.
    ///
    /// This command will attempt to load a document from storage by its ID.
    /// If found, the document will be loaded into memory and made available
    /// for operations.
    ///
    /// # Arguments
    ///
    /// * `document_id` - The ID of the document to find
    ///
    /// # Returns
    ///
    /// A `DispatchedCommand` containing the command ID and event.
    pub fn find_document(document_id: DocumentId) -> DispatchedCommand {
        Self::dispatch_command(Command::FindDocument { document_id })
    }

    /// Creates an event indicating that a network connection has been lost externally.
    ///
    /// This event is used when the calling application detects that a network connection
    /// has been lost (e.g., TCP connection drop, network failure). Unlike internal
    /// disconnections initiated by Samod, this represents an external connection loss
    /// that doesn't require an IoTask to be issued.
    ///
    /// The connection will be marked as failed and cleaned up internally, and a
    /// ConnectionFailed event will be emitted.
    ///
    /// # Arguments
    ///
    /// * `connection_id` - The ID of the connection that was lost
    pub fn connection_lost(connection_id: ConnectionId) -> Self {
        HubEvent {
            payload: HubEventPayload::Input(HubInput::ConnectionLost { connection_id }),
        }
    }

    /// Internal helper to create a dispatched command with a unique ID.
    ///
    /// This method wraps a command in the necessary structures to track
    /// its execution and completion.
    ///
    /// # Arguments
    ///
    /// * `command` - The command to be dispatched
    ///
    /// # Returns
    ///
    /// A `DispatchedCommand` containing both the command ID and event
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

    pub fn stop() -> HubEvent {
        HubEvent {
            payload: HubEventPayload::Input(HubInput::Stop),
        }
    }
}

impl std::fmt::Display for HubEvent {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:?}", self.payload)
    }
}
