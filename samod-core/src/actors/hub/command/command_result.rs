use crate::{ConnectionId, DocumentActorId, DocumentId};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CommandResult {
    /// Connection created, handshake initiated if outgoing.
    CreateConnection {
        connection_id: ConnectionId,
    },
    DisconnectConnection,
    /// Message received and processed.
    Receive {
        connection_id: ConnectionId,
        /// Any protocol errors that occurred
        error: Option<String>,
    },
    /// Result of ActorReady command.
    ActorReady,
    /// Result of CreateDocument command.
    CreateDocument {
        actor_id: DocumentActorId,
        document_id: DocumentId,
    },
    /// Result of FindDocument command.
    FindDocument {
        actor_id: DocumentActorId,
        found: bool,
    },
}
