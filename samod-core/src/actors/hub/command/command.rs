use automerge::Automerge;

use crate::{ConnectionId, DocumentId, network::ConnDirection};

/// Represents high-level operations that can be performed by the samod-core system.
///
/// Commands are the primary way to request actions from samod-core. They are typically
/// created through `HubEvent` static methods and executed asynchronously by the internal
/// future runtime. Each command returns a specific `CommandResult` when completed.
///
/// ## Command Lifecycle
///
/// 1. Commands are created via `Event` methods (e.g., `Event::create_connection(ConnDirection::Outgoing)`)
/// 2. They are assigned unique `CommandId`s and wrapped in `DispatchedCommand`
/// 3. The command is processed asynchronously by internal futures
/// 4. Results are returned via `HubResults::completed_commands`
#[derive(Clone)]
pub(crate) enum Command {
    /// Creates a new network connection and begins handshake if outgoing.
    ///
    /// This command establishes a new communication channel that can be used
    /// for sending and receiving messages. The command completes with a
    /// `CommandResult::CreateConnection` containing the new connection's ID.
    ///
    /// ## Fields
    ///
    /// * `direction` - Whether this is an outgoing or incoming connection
    ///
    /// ## Usage
    ///
    /// Connections are the primary mechanism for network communication in samod-core.
    /// Once created, connections can be used with `Send` actions and `Receive` commands.
    CreateConnection { direction: ConnDirection },

    /// Processes an incoming message, handling handshake or sync messages.
    ///
    /// This command handles messages received from external sources on an
    /// established connection. It will process handshake messages during
    /// the connection establishment phase, and document sync messages once
    /// the handshake is complete.
    ///
    /// ## Fields
    ///
    /// * `connection_id` - The ID of the connection on which the message was received
    /// * `msg` - The message content as raw bytes
    Receive {
        connection_id: ConnectionId,
        msg: Vec<u8>,
    },

    /// Indicates that a document actor is ready to process messages.
    ///
    /// This command is sent when a document actor has finished initializing
    /// and is ready to handle sync messages and other operations.
    ///
    /// ## Fields
    ///
    /// * `document_id` - The ID of the document that the actor manages
    ActorReady { document_id: DocumentId },

    /// Creates a new document.
    ///
    /// This command creates a new Automerge document with a generated ID.
    /// The document is initialized with an empty change and saved to storage.
    /// The command completes with a `CommandResult::CreateDocument` containing
    /// the new document's ID.
    CreateDocument { content: Box<Automerge> },

    /// Finds and loads an existing document.
    ///
    /// This command attempts to load a document from storage by its ID.
    /// If the document exists, it is loaded into memory and made available
    /// for operations. The command completes with a `CommandResult::FindDocument`
    /// indicating whether the document was found.
    ///
    /// ## Fields
    ///
    /// * `document_id` - The ID of the document to find
    FindDocument { document_id: DocumentId },
}

impl std::fmt::Debug for Command {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Command::CreateConnection { direction } => f
                .debug_struct("CreateConnection")
                .field("direction", direction)
                .finish(),
            Command::Receive { connection_id, msg } => f
                .debug_struct("Receive")
                .field("connection_id", connection_id)
                .field("msg(bytes)", &msg.len())
                .finish(),
            Command::ActorReady { document_id } => f
                .debug_struct("ActorReady")
                .field("document_id", document_id)
                .finish(),
            Command::CreateDocument { content: _ } => f
                .debug_struct("CreateDocument")
                .field("content", &"<Automerge>")
                .finish(),
            Command::FindDocument { document_id } => f
                .debug_struct("FindDocument")
                .field("document_id", document_id)
                .finish(),
        }
    }
}
