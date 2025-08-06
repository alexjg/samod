use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};

use crate::{
    ConnectionId, DocumentActorId, DocumentId, PeerId, UnixTimestamp,
    actors::{
        document::DocumentStatus,
        hub::{CommandId, CommandResult, State, connection::Connection, state::ActorInfo},
    },
    ephemera::EphemeralMessage,
    network::{ConnectionInfo, PeerDocState, PeerMetadata},
};
use futures::channel::oneshot;

use super::{ConnectionAccess, IoAccess};

pub(crate) struct StateAccess<'a> {
    now: UnixTimestamp,
    io: IoAccess,
    state: &'a Arc<Mutex<State>>,
}

impl<'a> StateAccess<'a> {
    pub(crate) fn new(now: UnixTimestamp, io: IoAccess, state: &'a Arc<Mutex<State>>) -> Self {
        Self { now, io, state }
    }

    pub(crate) fn add_connection(&self, connection_id: ConnectionId, connection_state: Connection) {
        self.state
            .lock()
            .unwrap()
            .add_connection(connection_id, connection_state);
    }

    pub(crate) fn remove_connection(&self, connection_id: &ConnectionId) -> Option<Connection> {
        self.state.lock().unwrap().remove_connection(connection_id)
    }

    pub(crate) fn get_connection(
        &self,
        connection_id: &ConnectionId,
    ) -> Option<ConnectionAccess<'a>> {
        if self
            .state
            .lock()
            .unwrap()
            .get_connection(connection_id)
            .is_some()
        {
            Some(ConnectionAccess {
                now: self.now,
                io: self.io.clone(),
                state: self.state,
                conn_id: *connection_id,
            })
        } else {
            None
        }
    }

    /// Get the peer ID for this samod instance
    pub(crate) fn peer_id(&self) -> PeerId {
        self.state.lock().unwrap().peer_id().clone()
    }

    /// Get local metadata to send during handshake
    pub(crate) fn get_local_metadata(&self) -> PeerMetadata {
        self.state.lock().unwrap().get_local_metadata()
    }

    pub(crate) fn add_document_actor(&self, actor_id: DocumentActorId, document_id: DocumentId) {
        self.state
            .lock()
            .unwrap()
            .add_document_actor(actor_id, document_id);
    }

    pub(crate) fn remove_document_actor(&self, actor_id: DocumentActorId) {
        self.state.lock().unwrap().remove_document_actor(&actor_id);
    }

    pub(crate) fn find_actor_for_document(&self, document_id: &DocumentId) -> Option<ActorInfo> {
        self.state
            .lock()
            .unwrap()
            .find_actor_for_document(document_id)
            .cloned()
    }

    pub(crate) fn document_actors(&self) -> Vec<ActorInfo> {
        self.state
            .lock()
            .unwrap()
            .document_actors()
            .cloned()
            .collect()
    }

    pub(crate) fn add_pending_find_command(
        &self,
        document_id: DocumentId,
        command_id: CommandId,
        reply: oneshot::Sender<CommandResult>,
    ) {
        self.state
            .lock()
            .unwrap()
            .add_pending_find_command(document_id, command_id, reply);
    }

    pub(crate) fn add_pending_create_command(
        &self,
        actor_id: DocumentActorId,
        command_id: CommandId,
        reply: oneshot::Sender<CommandResult>,
    ) {
        self.state
            .lock()
            .unwrap()
            .add_pending_create_command(actor_id, command_id, reply);
    }

    /// Get a list of all established peer connections
    pub(crate) fn established_peers(&self) -> Vec<(ConnectionId, PeerId)> {
        self.state.lock().unwrap().established_peers()
    }

    pub(crate) fn remote_peer_id(&self, conn_id: ConnectionId) -> Option<PeerId> {
        self.state
            .lock()
            .unwrap()
            .get_connection(&conn_id)
            .and_then(|c| c.remote_peer_id())
            .cloned()
    }

    pub(crate) fn update_document_status(
        &self,
        doc_actor: DocumentActorId,
        new_status: DocumentStatus,
    ) {
        self.state
            .lock()
            .unwrap()
            .update_document_status(doc_actor, new_status)
    }

    pub(crate) fn add_document_to_connection(
        &self,
        connection_id: &ConnectionId,
        document_id: DocumentId,
    ) {
        self.state
            .lock()
            .unwrap()
            .add_document_to_connection(connection_id, document_id);
    }

    pub(crate) fn ensure_connections(&self) -> Vec<(DocumentActorId, ConnectionId, PeerId)> {
        self.state.lock().unwrap().ensure_connections()
    }

    pub(crate) fn pop_closed_connections(&self) -> Vec<ConnectionId> {
        self.state.lock().unwrap().pop_closed_connections()
    }

    pub(crate) fn update_peer_states(
        &self,
        actor: DocumentActorId,
        new_states: HashMap<ConnectionId, PeerDocState>,
    ) {
        self.state
            .lock()
            .unwrap()
            .update_peer_states(actor, new_states);
    }

    pub(crate) fn pop_new_connection_info(&self) -> HashMap<ConnectionId, ConnectionInfo> {
        self.state.lock().unwrap().pop_new_connection_info()
    }

    pub(crate) fn receive_ephemeral_message(
        &self,
        msg: EphemeralMessage,
    ) -> Option<EphemeralMessage> {
        self.state.lock().unwrap().receive_ephemeral_msg(msg)
    }
}
