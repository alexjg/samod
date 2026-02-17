use automerge::Automerge;

use crate::{ConnectionId, DocumentId};

/// Represents high-level operations that can be performed by the samod-core system.
///
/// Commands are the primary way to request actions from samod-core. They are typically
/// created through `HubEvent` static methods and executed asynchronously by the internal
/// future runtime. Each command returns a specific `CommandResult` when completed.
///
/// ## Command Lifecycle
///
/// 1. Commands are created via `Event` methods (e.g., `Event::create_document()`)
/// 2. They are assigned unique `CommandId`s and wrapped in `DispatchedCommand`
/// 3. The command is submitted to the hub via [`Hub::handle_event()`]
/// 4. Results are returned via [`HubResults::completed_commands`]
#[derive(Clone)]
pub(crate) enum Command {
    /// Processes an incoming message, handling handshake or sync messages.
    Receive {
        connection_id: ConnectionId,
        msg: Vec<u8>,
    },

    /// Indicates that a document actor is ready to process messages.
    ActorReady { document_id: DocumentId },

    /// Creates a new document.
    CreateDocument { content: Box<Automerge> },

    /// Finds and loads an existing document.
    FindDocument { document_id: DocumentId },
}

impl std::fmt::Debug for Command {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
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
