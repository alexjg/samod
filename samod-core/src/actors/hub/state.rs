use futures::channel::oneshot;
use std::collections::HashMap;

use crate::{
    ConnectionId, DocumentActorId, DocumentId, PeerId, StorageId,
    actors::document::DocumentStatus,
    ephemera::{EphemeralMessage, EphemeralSession, OutgoingSessionDetails},
    network::{ConnectionInfo, ConnectionState, PeerDocState, PeerMetadata},
};

mod actor_info;
pub(crate) use actor_info::ActorInfo;

use super::{CommandId, CommandResult, RunState, connection::Connection};
mod pending_commands;

/// `State` holds all shared mutable state that needs to be accessed by
/// command handler futures during execution. It is wrapped in `Rc<RefCell<_>>`
/// to allow shared access while maintaining Rust's borrowing rules.
///
/// ## Access Pattern
///
/// State is accessed through the `TaskContext::state()` method, which returns
/// a `StateAccess` guard that ensures borrows are properly scoped and cannot
/// be held across await points.
///
/// ## Example Usage
///
/// State is accessed through TaskContext in command handlers:
/// ```text
/// // Check if connection exists
/// let connection_exists = ctx.state().has_connection_id(&connection_id);
///
/// if !connection_exists {
///     // Create new connection
///     ctx.state().add_connection_id(connection_id);
/// }
/// ```
pub(crate) struct State {
    /// The storage ID that identifies this peer's storage layer.
    ///
    /// This ID identifies the storage layer that this peer is connected to.
    /// Multiple peers may share the same storage ID when they're connected to
    /// the same underlying storage (e.g., tabs sharing IndexedDB, processes
    /// sharing filesystem storage).
    pub(crate) storage_id: StorageId,

    /// The unique peer ID for this samod instance.
    pub(crate) peer_id: PeerId,

    /// Active document actors
    actors: HashMap<DocumentActorId, ActorInfo>,

    /// Connection state for each connection
    connections: HashMap<ConnectionId, Connection>,

    /// Map from document ID to actor ID for quick lookups
    document_to_actor: HashMap<DocumentId, DocumentActorId>,

    // Commands we are currently processing
    pending_commands: pending_commands::PendingCommands,

    ephemeral_session: EphemeralSession,

    run_state: RunState,
}

impl State {
    pub(crate) fn new(
        storage_id: StorageId,
        peer_id: PeerId,
        ephemeral_session: EphemeralSession,
    ) -> Self {
        Self {
            storage_id,
            peer_id,
            actors: HashMap::new(),
            connections: HashMap::new(),
            document_to_actor: HashMap::new(),
            pending_commands: pending_commands::PendingCommands::new(),
            ephemeral_session,
            run_state: RunState::Running,
        }
    }

    /// Returns the current storage ID if it has been loaded.
    pub(crate) fn storage_id(&self) -> StorageId {
        self.storage_id.clone()
    }

    pub(crate) fn add_connection(
        &mut self,
        connection_id: ConnectionId,
        connection_state: Connection,
    ) {
        self.connections.insert(connection_id, connection_state);
    }

    pub(crate) fn remove_connection(&mut self, connection_id: &ConnectionId) -> Option<Connection> {
        self.connections.remove(connection_id)
    }

    pub(crate) fn get_connection(&self, connection_id: &ConnectionId) -> Option<&Connection> {
        self.connections.get(connection_id)
    }

    pub(crate) fn get_connection_mut(
        &mut self,
        connection_id: &ConnectionId,
    ) -> Option<&mut Connection> {
        self.connections.get_mut(connection_id)
    }

    pub(crate) fn add_document_to_connection(
        &mut self,
        connection_id: &ConnectionId,
        document_id: DocumentId,
    ) {
        if let Some(connection) = self.connections.get_mut(connection_id) {
            connection.add_document(document_id);
        }
    }

    /// Get the peer ID for this samod instance
    pub(crate) fn peer_id(&self) -> &PeerId {
        &self.peer_id
    }

    /// Get a list of all connection IDs
    pub(crate) fn connections(&self) -> Vec<ConnectionInfo> {
        self.connections
            .iter()
            .map(|(conn_id, conn)| {
                let (doc_connections, state) =
                    if let Some(established) = conn.established_connection() {
                        (
                            established.document_subscriptions().clone(),
                            ConnectionState::Connected {
                                their_peer_id: established.remote_peer_id().clone(),
                            },
                        )
                    } else {
                        (HashMap::new(), ConnectionState::Handshaking)
                    };
                ConnectionInfo {
                    id: *conn_id,
                    last_received: conn.last_received(),
                    last_sent: conn.last_sent(),
                    docs: doc_connections,
                    state,
                }
            })
            .collect()
    }

    /// Get a list of all established peer connections
    pub(crate) fn established_peers(&self) -> Vec<(ConnectionId, PeerId)> {
        self.connections
            .iter()
            .filter_map(|(connection_id, connection_state)| {
                connection_state
                    .remote_peer_id()
                    .map(|remote| (*connection_id, remote.clone()))
            })
            .collect()
    }

    /// Check if connected to a specific peer
    pub(crate) fn is_connected_to(&self, peer_id: &PeerId) -> bool {
        self.connections.values().any(|connection_state| {
            connection_state
                .established_connection()
                .map(|established| established.remote_peer_id() == peer_id)
                .unwrap_or(false)
        })
    }

    /// Adds a document actor to the state.
    ///
    /// This method registers both the actor handle and the document-to-actor mapping.
    pub(crate) fn add_document_actor(
        &mut self,
        actor_id: DocumentActorId,
        document_id: DocumentId,
    ) {
        let handle = ActorInfo::new_with_id(actor_id, document_id.clone());
        self.actors.insert(actor_id, handle);
        self.document_to_actor.insert(document_id, actor_id);
    }

    pub(crate) fn remove_document_actor(&mut self, actor_id: &DocumentActorId) {
        if let Some(actor_info) = self.actors.remove(actor_id) {
            self.document_to_actor.remove(&actor_info.document_id);
        } else {
            tracing::warn!(
                "Attempted to remove non-existent document actor: {:?}",
                actor_id
            );
        }
    }

    pub(crate) fn find_actor_for_document(&self, document_id: &DocumentId) -> Option<&ActorInfo> {
        self.document_to_actor
            .get(document_id)
            .and_then(|actor_id| self.actors.get(actor_id))
    }

    pub(crate) fn find_document_for_actor(&self, actor_id: &DocumentActorId) -> Option<DocumentId> {
        self.actors
            .get(actor_id)
            .map(|actor| actor.document_id.clone())
    }

    /// Adds a command ID to the list of commands waiting for a document operation to complete.
    pub(crate) fn add_pending_find_command(
        &mut self,
        document_id: DocumentId,
        command_id: CommandId,
        reply: oneshot::Sender<CommandResult>,
    ) {
        self.pending_commands
            .add_pending_find_command(document_id, command_id, reply);
    }

    /// Adds a command ID to the list of commands waiting for an actor to report readiness.
    pub(crate) fn add_pending_create_command(
        &mut self,
        actor_id: DocumentActorId,
        command_id: CommandId,
        reply: oneshot::Sender<CommandResult>,
    ) {
        self.pending_commands
            .add_pending_create_command(actor_id, command_id, reply);
    }

    pub(crate) fn document_actors(&self) -> impl Iterator<Item = &ActorInfo> {
        self.actors.values()
    }

    pub(crate) fn update_document_status(
        &mut self,
        actor_id: DocumentActorId,
        new_status: DocumentStatus,
    ) {
        let Some(actor_info) = self.actors.get_mut(&actor_id) else {
            tracing::warn!("document actor ID not found in actors: {:?}", actor_id);
            return;
        };
        actor_info.status = new_status;
        let doc_id = actor_info.document_id.clone();
        match new_status {
            DocumentStatus::Ready => {
                self.pending_commands
                    .resolve_pending_create(actor_id, &doc_id);
                self.pending_commands
                    .resolve_pending_find(&doc_id, actor_id, true);
            }
            DocumentStatus::NotFound => {
                assert!(!self.pending_commands.has_pending_create(actor_id));
                self.pending_commands
                    .resolve_pending_find(&doc_id, actor_id, false);
            }
            _ => {}
        }
    }

    pub(crate) fn ensure_connections(&mut self) -> Vec<(DocumentActorId, ConnectionId, PeerId)> {
        let mut to_connect = Vec::new();
        for (conn_id, conn) in &mut self.connections {
            if let Some(established) = conn.established_connection_mut() {
                for (doc_id, doc_actor) in &self.document_to_actor {
                    if !established.document_subscriptions().contains_key(doc_id) {
                        to_connect.push((
                            established.remote_peer_id().clone(),
                            *conn_id,
                            doc_id.clone(),
                            doc_actor,
                        ));
                        established.add_document_subscription(doc_id.clone());
                    }
                }
            }
        }

        let mut result = Vec::new();

        for (peer_id, conn_id, doc_id, actor) in to_connect {
            let conn = self.connections.get_mut(&conn_id).unwrap();
            conn.add_document(doc_id);
            result.push((*actor, conn_id, peer_id));
        }

        result
    }

    pub(crate) fn update_peer_states(
        &mut self,
        actor: DocumentActorId,
        new_states: HashMap<ConnectionId, PeerDocState>,
    ) {
        let Some(actor) = self.actors.get(&actor) else {
            tracing::warn!(
                ?actor,
                "document actor ID not found in actors when updating peer states"
            );
            return;
        };
        for (conn, new_state) in new_states {
            if let Some(connection) = self.connections.get_mut(&conn) {
                connection.update_peer_state(&actor.document_id, new_state);
            } else {
                tracing::warn!(?conn, "connection not found when updating peer states");
            }
        }
    }

    pub(crate) fn pop_closed_connections(&mut self) -> Vec<ConnectionId> {
        let closed: Vec<_> = self
            .connections
            .iter()
            .filter_map(|(id, conn)| if conn.is_closed() { Some(*id) } else { None })
            .collect();

        for id in &closed {
            self.connections.remove(id);
        }

        closed
    }

    pub(crate) fn pop_new_connection_info(&mut self) -> HashMap<ConnectionId, ConnectionInfo> {
        self.connections
            .iter_mut()
            .filter_map(|(conn_id, conn)| conn.pop_new_info().map(|info| (*conn_id, info)))
            .collect()
    }

    pub(crate) fn next_ephemeral_msg_details(&mut self) -> OutgoingSessionDetails {
        self.ephemeral_session.next_message_session_details()
    }

    pub(crate) fn receive_ephemeral_msg(
        &mut self,
        msg: EphemeralMessage,
    ) -> Option<EphemeralMessage> {
        self.ephemeral_session.receive_message(msg)
    }

    pub(crate) fn get_local_metadata(&self) -> PeerMetadata {
        PeerMetadata {
            is_ephemeral: false,
        }
    }

    pub(crate) fn run_state(&self) -> RunState {
        self.run_state
    }

    pub(crate) fn set_run_state(&mut self, new_state: RunState) {
        self.run_state = new_state;
    }
}
