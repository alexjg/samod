use std::collections::HashMap;

use crate::{
    ConnectionId, DocumentActorId, DocumentId, PeerId, StorageId, UnixTimestamp,
    actors::{
        document::{DocumentStatus, SpawnArgs},
        hub::{
            Command, HubEvent, HubEventPayload, HubInput, HubResults,
            connection::{ConnectionArgs, ReceiveEvent},
            io::{HubIoAction, HubIoResult},
        },
        messages::{Broadcast, DocMessage, DocToHubMsgPayload, HubToDocMsgPayload},
    },
    ephemera::{EphemeralMessage, EphemeralSession, OutgoingSessionDetails},
    network::{
        ConnDirection, ConnectionEvent, ConnectionInfo, ConnectionState, PeerDocState, PeerInfo,
        PeerMetadata,
        wire_protocol::{WireMessage, WireMessageBuilder},
    },
};

mod actor_info;
pub(crate) use actor_info::ActorInfo;
use automerge::Automerge;

use super::{CommandId, CommandResult, RunState, connection::Connection};
mod pending_commands;

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

    pub(crate) fn established_connection(
        &mut self,
        conn_id: ConnectionId,
    ) -> Option<(&mut Connection, PeerId)> {
        let conn = self.connections.get_mut(&conn_id)?;
        if let Some(peer_id) = conn.remote_peer_id().cloned() {
            Some((conn, peer_id))
        } else {
            None
        }
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
    ) {
        self.pending_commands
            .add_pending_find_command(document_id, command_id);
    }

    /// Adds a command ID to the list of commands waiting for an actor to report readiness.
    pub(crate) fn add_pending_create_command(
        &mut self,
        actor_id: DocumentActorId,
        command_id: CommandId,
    ) {
        self.pending_commands
            .add_pending_create_command(actor_id, command_id);
    }

    pub(crate) fn pop_completed_commands(&mut self) -> Vec<(CommandId, CommandResult)> {
        self.pending_commands.pop_completed_commands()
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

    pub(crate) fn get_local_metadata(&self) -> PeerMetadata {
        PeerMetadata {
            is_ephemeral: false,
        }
    }

    pub(crate) fn run_state(&self) -> RunState {
        self.run_state
    }

    pub(crate) fn handle_event<R: rand::Rng>(
        &mut self,
        rng: &mut R,
        now: UnixTimestamp,
        event: HubEvent,
        results: &mut HubResults,
    ) {
        assert!(self.run_state != RunState::Stopped);
        match event.payload {
            HubEventPayload::IoComplete(io_completion) => {
                match io_completion.payload {
                    HubIoResult::Send | HubIoResult::Disconnect => {
                        // Nothing to do here
                    }
                }
            }
            HubEventPayload::Input(input) => {
                match input {
                    HubInput::Stop => {
                        if self.run_state == RunState::Running {
                            tracing::info!("stopping hub event loop");
                            self.run_state = RunState::Stopping;
                            for actor_info in self.document_actors() {
                                // Notify all document actors that we're stopping
                                results.send_to_doc_actor(
                                    actor_info.actor_id,
                                    HubToDocMsgPayload::Terminate,
                                );
                            }
                        }
                    }
                    HubInput::Command {
                        command_id,
                        command,
                    } => {
                        if self.run_state == RunState::Running
                            && let Some(result) =
                                self.handle_command(rng, now, results, command_id, *command)
                        {
                            results.completed_commands.insert(command_id, result);
                        }
                    }
                    HubInput::Tick => {
                        // Tick events are used to trigger periodic processing
                        // but don't spawn new command futures
                    }
                    HubInput::ActorMessage { actor_id, message } => match message {
                        DocToHubMsgPayload::DocumentStatusChanged { new_status } => {
                            self.update_document_status(actor_id, new_status);
                        }
                        DocToHubMsgPayload::SendSyncMessage {
                            document_id,
                            connection_id,
                            message,
                        } => {
                            let sender_id = self.peer_id.clone();
                            if let Some((conn, target_id)) =
                                self.established_connection(connection_id)
                            {
                                let wire_message = WireMessageBuilder {
                                    sender_id,
                                    target_id,
                                    document_id,
                                }
                                .from_sync_message(message);
                                results.send(conn, wire_message.encode());
                            } else {
                                tracing::warn!(
                                    ?connection_id,
                                    "received SendSyncMessage for unknown connection ID"
                                );
                            }
                        }
                        DocToHubMsgPayload::PeerStatesChanged { new_states } => {
                            self.update_peer_states(actor_id, new_states);
                        }
                        DocToHubMsgPayload::Broadcast { connections, msg } => {
                            self.broadcast(results, actor_id, connections, msg);
                        }
                        DocToHubMsgPayload::Terminated => {
                            tracing::debug!(?actor_id, "document actor terminated");
                            self.actors.remove(&actor_id);
                        }
                    },
                    HubInput::ConnectionLost { connection_id } => {
                        if let Some(_connection) = self.remove_connection(&connection_id) {
                            results.emit_disconnect_event(
                                connection_id,
                                "Connection lost externally".to_string(),
                            );
                            self.notify_doc_actors_of_removed_connection(results, connection_id);
                        }
                    }
                }
            }
        }

        // Notify document actors of any closed connections
        for conn_id in self.pop_closed_connections() {
            for doc in self.document_actors() {
                results.send_to_doc_actor(
                    doc.actor_id,
                    HubToDocMsgPayload::ConnectionClosed {
                        connection_id: conn_id,
                    },
                );
            }
        }

        // Now ensure that every connection is connected to every document
        if self.run_state == RunState::Running {
            for (actor_id, conn_id, peer_id) in self.ensure_connections() {
                results.send_to_doc_actor(
                    actor_id,
                    HubToDocMsgPayload::NewConnection {
                        connection_id: conn_id,
                        peer_id,
                    },
                );
            }
        }

        // Notify any listeners of updated connection info ("info" here is for monitoring purposes,
        // things like the last time we sent a message and the heads of each document according
        // to the connection and so on).
        for (conn_id, new_state) in self.pop_new_connection_info() {
            results.emit_connection_event(ConnectionEvent::StateChanged {
                connection_id: conn_id,
                new_state,
            });
        }

        for (command_id, result) in self.pop_completed_commands() {
            results.completed_commands.insert(command_id, result);
        }

        if self.run_state == RunState::Stopping {
            if self.actors.is_empty() {
                tracing::debug!("hub stopped");
                self.run_state = RunState::Stopped;
            } else {
                tracing::debug!(remaining_actors = self.actors.len(), "hub still stopping");
            }
        }

        results.stopped = self.run_state == RunState::Stopped;
    }

    /// Handle a command, returning `Some(CommandResult)` if the command was handled
    /// immediately and `None` if it will be completed asynchronously
    fn handle_command<R: rand::Rng>(
        &mut self,
        rng: &mut R,
        now: UnixTimestamp,
        out: &mut HubResults,
        command_id: CommandId,
        command: Command,
    ) -> Option<CommandResult> {
        match command {
            Command::CreateConnection { direction } => {
                Some(self.handle_create_connection(now, out, direction))
            }
            Command::Receive { connection_id, msg } => {
                Some(self.handle_receive(now, out, connection_id, msg))
            }
            Command::ActorReady { document_id: _ } => Some(CommandResult::ActorReady),
            Command::CreateDocument { content } => {
                self.handle_create_document(rng, out, command_id, *content);
                None
            }
            Command::FindDocument { document_id } => {
                self.handle_find_document(out, command_id, document_id)
            }
        }
    }

    fn handle_create_connection(
        &mut self,
        now: UnixTimestamp,
        out: &mut HubResults,
        direction: ConnDirection,
    ) -> CommandResult {
        let local_peer_id = self.peer_id.clone();
        let local_metadata = self.get_local_metadata();

        let connection = Connection::new_handshaking(
            out,
            ConnectionArgs {
                direction,
                local_peer_id: local_peer_id.clone(),
                local_metadata: Some(local_metadata.clone()),
                created_at: now,
            },
        );

        let connection_id = connection.id();

        tracing::debug!(?connection_id, ?direction, "creating new connection");

        self.add_connection(connection_id, connection);

        CommandResult::CreateConnection { connection_id }
    }

    fn handle_receive(
        &mut self,
        now: UnixTimestamp,
        out: &mut HubResults,
        connection_id: ConnectionId,
        msg: Vec<u8>,
    ) -> CommandResult {
        tracing::trace!(?connection_id, msg_bytes = msg.len(), "receive");
        let Some(conn) = self.connections.get_mut(&connection_id) else {
            tracing::warn!(?connection_id, "receive command for nonexistent connection");

            return CommandResult::Receive {
                connection_id,
                error: Some("Connection not found".to_string()),
            };
        };

        let msg = match WireMessage::decode(&msg) {
            Ok(msg) => msg,
            Err(e) => {
                tracing::warn!(
                    ?connection_id,
                    err=?e,
                    "failed to decode message: {}",
                    e
                );
                let error_msg = format!("Message decode error: {e}");
                self.fail_connection_with_disconnect(out, connection_id, error_msg);

                return CommandResult::Receive {
                    connection_id,
                    error: Some(format!("Decode error: {e}")),
                };
            }
        };

        for evt in conn.receive_msg(out, now, msg) {
            match evt {
                ReceiveEvent::HandshakeComplete { remote_peer_id } => {
                    tracing::debug!(?connection_id, ?remote_peer_id, "handshake completed");
                    // Emit handshake completed event
                    let peer_info = PeerInfo {
                        peer_id: remote_peer_id.clone(),
                        metadata: Some(self.get_local_metadata()),
                        protocol_version: "1".to_string(),
                    };
                    out.emit_connection_event(ConnectionEvent::HandshakeCompleted {
                        connection_id,
                        peer_info: peer_info.clone(),
                    })
                }
                ReceiveEvent::SyncMessage {
                    doc_id,
                    sender_id: _,
                    target_id,
                    msg,
                } => self.handle_doc_message(
                    out,
                    connection_id,
                    target_id,
                    doc_id,
                    DocMessage::Sync(msg),
                ),
                ReceiveEvent::EphemeralMessage {
                    doc_id,
                    sender_id,
                    target_id,
                    count,
                    session_id,
                    msg,
                } => {
                    let msg = EphemeralMessage {
                        sender_id,
                        session_id,
                        count,
                        data: msg,
                    };
                    if let Some(msg) = self.ephemeral_session.receive_message(msg) {
                        self.handle_doc_message(
                            out,
                            connection_id,
                            target_id,
                            doc_id,
                            DocMessage::Ephemeral(msg),
                        )
                    }
                }
            }
        }
        CommandResult::Receive {
            connection_id,
            error: None,
        }
    }

    fn handle_doc_message(
        &mut self,
        out: &mut HubResults,
        connection_id: ConnectionId,
        target_id: PeerId,
        doc_id: DocumentId,
        msg: DocMessage,
    ) {
        // Validate this request is for us
        if target_id != self.peer_id {
            tracing::trace!(?connection_id, ?msg, "ignoring message for another peer");
        }

        // Ensure there's a document actor for this document
        if let Some(existing_actor) = self.find_actor_for_document(&doc_id) {
            // Forward the request to the document actor
            out.send_to_doc_actor(
                existing_actor.actor_id,
                HubToDocMsgPayload::HandleDocMessage {
                    connection_id,
                    message: msg,
                },
            );
        } else {
            self.spawn_actor(out, doc_id, None, Some((connection_id, msg)));
        }
    }

    #[tracing::instrument(skip(self, init_doc, rng), fields(command_id = %command_id))]
    fn handle_create_document<R: rand::Rng>(
        &mut self,
        rng: &mut R,
        out: &mut HubResults,
        command_id: CommandId,
        init_doc: Automerge,
    ) {
        // Generate new document ID
        let document_id = DocumentId::new(rng);

        tracing::debug!(%document_id, "creating new document");

        let actor_id = self.spawn_actor(out, document_id, Some(init_doc), None);

        // Queue command for completion when actor reports ready
        self.add_pending_create_command(actor_id, command_id);
    }

    #[tracing::instrument(skip(self, out), fields(document_id = %document_id))]
    fn handle_find_document(
        &mut self,
        out: &mut HubResults,
        command_id: CommandId,
        document_id: DocumentId,
    ) -> Option<CommandResult> {
        tracing::debug!("find document command received");
        // Check if actor already exists and is ready
        if let Some(actor_info) = self.find_actor_for_document(&document_id) {
            tracing::trace!(%actor_info.actor_id, ?actor_info.status, "found existing actor for document");
            return match actor_info.status {
                DocumentStatus::Spawned | DocumentStatus::Requesting | DocumentStatus::Loading => {
                    self.add_pending_find_command(document_id, command_id);
                    None
                }
                DocumentStatus::Ready => {
                    // Document is ready
                    Some(CommandResult::FindDocument {
                        found: true,
                        actor_id: actor_info.actor_id,
                    })
                }
                DocumentStatus::NotFound => {
                    // In this case we need to restart the request process
                    tracing::trace!(%actor_info.actor_id, ?actor_info.status, "re-requesting document from actor");
                    out.send_to_doc_actor(actor_info.actor_id, HubToDocMsgPayload::RequestAgain);

                    self.add_pending_find_command(document_id, command_id);
                    None
                }
            };
        }

        tracing::trace!("no existing actor found for document, spawning new actor");

        self.spawn_actor(out, document_id.clone(), None, None);

        self.add_pending_find_command(document_id, command_id);
        None
    }

    fn fail_connection_with_disconnect(
        &mut self,
        out: &mut HubResults,
        connection_id: crate::ConnectionId,
        error: String,
    ) {
        let Some(connection) = self.remove_connection(&connection_id) else {
            tracing::warn!(
                ?connection_id,
                "attempting to fail a connection that does not exist"
            );
            return;
        };
        tracing::debug!(?error, remote_peer_id=?connection.remote_peer_id(), "failing connection");

        out.emit_disconnect_event(connection_id, error);
        self.notify_doc_actors_of_removed_connection(out, connection_id);
        // Emit disconnect IoTask so caller can clean up the network connection
        out.emit_io_action(HubIoAction::Disconnect { connection_id });
    }

    fn notify_doc_actors_of_removed_connection(
        &mut self,
        out: &mut HubResults,
        connection_id: crate::ConnectionId,
    ) {
        for actor_info in self.document_actors() {
            out.send_to_doc_actor(
                actor_info.actor_id,
                HubToDocMsgPayload::ConnectionClosed { connection_id },
            );
        }
    }

    fn spawn_actor(
        &mut self,
        out: &mut HubResults,
        document_id: DocumentId,
        initial_doc: Option<Automerge>,
        from_sync_msg: Option<(ConnectionId, DocMessage)>,
    ) -> DocumentActorId {
        // Create new actor to find/load the document
        let actor_id = DocumentActorId::new();

        // Create the actor and initialize it
        self.add_document_actor(actor_id, document_id.clone());

        let mut initial_connections: HashMap<ConnectionId, (PeerId, Option<DocMessage>)> = self
            .established_peers()
            .iter()
            .map(|(c, p)| (*c, (p.clone(), None)))
            .collect();

        for conn in initial_connections.keys() {
            self.add_document_to_connection(conn, document_id.clone());
        }

        if let Some((conn_id, msg)) = from_sync_msg
            && let Some((_, sync_msg)) = initial_connections.get_mut(&conn_id)
        {
            *sync_msg = Some(msg);
        }

        out.emit_spawn_actor(SpawnArgs {
            actor_id,
            local_peer_id: self.peer_id.clone(),
            document_id,
            initial_content: initial_doc,
            initial_connections,
        });

        actor_id
    }

    fn broadcast(
        &mut self,
        out: &mut HubResults,
        from_actor: DocumentActorId,
        to_connections: Vec<ConnectionId>,
        msg: Broadcast,
    ) {
        let Some(doc_id) = self.find_document_for_actor(&from_actor) else {
            tracing::warn!(
                ?from_actor,
                "attempting to broadcast from an actor that does not exist"
            );
            return;
        };
        let OutgoingSessionDetails {
            counter,
            session_id,
        } = self.next_ephemeral_msg_details();

        for conn_id in to_connections {
            let Some(conn) = self.connections.get_mut(&conn_id) else {
                continue;
            };
            let Some(their_peer_id) = conn
                .established_connection()
                .map(|c| c.remote_peer_id().clone())
            else {
                continue;
            };
            let msg = match &msg {
                Broadcast::New { msg } => WireMessage::Ephemeral {
                    sender_id: self.peer_id.clone(),
                    target_id: their_peer_id,
                    count: counter,
                    session_id: session_id.to_string(),
                    document_id: doc_id.clone(),
                    data: msg.clone(),
                },
                Broadcast::Gossip {
                    msg:
                        EphemeralMessage {
                            sender_id,
                            session_id,
                            count,
                            data,
                        },
                } => WireMessage::Ephemeral {
                    sender_id: sender_id.clone(),
                    target_id: their_peer_id,
                    count: *count,
                    session_id: session_id.to_string(),
                    document_id: doc_id.clone(),
                    data: data.clone(),
                },
            };
            out.send(conn, msg.encode());
        }
    }
}
