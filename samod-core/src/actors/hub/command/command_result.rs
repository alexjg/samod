use crate::{ConnectionId, DialerId, DocSearch, DocumentActorId, DocumentId, ListenerId};

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
    /// Result of SearchForDoc command.
    ///
    /// Provides the actor ID plus a synchronous snapshot of the document's
    /// current search state. Subsequent state changes are delivered via
    /// [`HubResults::search_state_updates`](crate::actors::hub::HubResults::search_state_updates).
    SearchForDoc {
        actor_id: DocumentActorId,
        search_state: DocSearch,
    },
    /// Result of AddDialer command.
    AddDialer {
        dialer_id: DialerId,
    },
    /// Result of AddListener command.
    AddListener {
        listener_id: ListenerId,
    },
}
