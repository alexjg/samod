use automerge::Automerge;
use samod_core::{
    CommandId, ConnectionId, DocumentActorId, DocumentChanged, DocumentId, PeerId, StorageId,
    actors::{DocumentError, hub::HubEvent},
    network::ConnectionEvent,
    network::ConnectionInfo,
};

use crate::samod_id::SamodId;

use super::{SamodWrapper, running_doc_ids::RunningDocIds};

// A wrapper struct which provides aconvenience API to access a `SamodWrapper`
pub struct SamodRef<'a> {
    pub(super) samod_id: &'a SamodId,
    pub(super) network: &'a mut super::Network,
}

impl SamodRef<'_> {
    pub fn storage_id(&mut self) -> StorageId {
        self.wrapper().storage_id()
    }

    pub fn storage(&mut self) -> &std::collections::HashMap<samod_core::StorageKey, Vec<u8>> {
        self.wrapper().storage()
    }

    pub fn actor_count(&mut self) -> usize {
        self.wrapper().actor_count()
    }

    pub fn create_document(&mut self) -> RunningDocIds {
        self.wrapper().create_document(None)
    }

    pub fn create_document_with_content(&mut self, content: automerge::Automerge) -> RunningDocIds {
        self.wrapper().create_document(Some(content))
    }

    pub fn start_create_document(&mut self) -> CommandId {
        self.wrapper().start_create_document()
    }

    pub fn check_create_document_result(&mut self, command_id: CommandId) -> Option<RunningDocIds> {
        self.wrapper().check_create_document_result(command_id)
    }

    pub fn find_document(
        &mut self,
        doc_id: &samod_core::DocumentId,
    ) -> Option<samod_core::DocumentActorId> {
        let command_id = self.wrapper().start_find_document(doc_id);
        self.wrapper().handle_events();
        self.network.run_until_quiescent();
        self.wrapper()
            .check_find_document_result(command_id)
            .expect("find command never completed")
    }

    pub fn begin_find_document(&mut self, doc_id: &samod_core::DocumentId) -> CommandId {
        self.wrapper().start_find_document(doc_id)
    }

    pub fn check_find_document_result(
        &mut self,
        command_id: CommandId,
    ) -> Option<Option<samod_core::DocumentActorId>> {
        self.wrapper().check_find_document_result(command_id)
    }

    pub fn get_actor(
        &mut self,
        actor_id: &DocumentActorId,
    ) -> Option<&samod_core::actors::DocumentActor> {
        self.wrapper().get_actor(actor_id)
    }

    /// Returns the connection events captured during event processing.
    pub fn connection_events(&mut self) -> Vec<ConnectionEvent> {
        self.wrapper().connection_events().to_vec()
    }

    /// Clears all captured connection events.
    pub fn clear_connection_events(&mut self) {
        self.wrapper().clear_connection_events()
    }

    /// Returns a list of all connection IDs.
    pub fn connections(&mut self) -> Vec<ConnectionInfo> {
        self.wrapper().connections()
    }

    /// Returns a list of all established peer connections.
    pub fn established_peers(&mut self) -> Vec<(ConnectionId, PeerId)> {
        self.wrapper().established_peers()
    }

    /// Checks if this instance is connected to a specific peer.
    pub fn is_connected_to(&mut self, peer_id: &PeerId) -> bool {
        self.wrapper().is_connected_to(peer_id)
    }

    /// Returns the peer ID for this samod instance.
    pub fn peer_id(&mut self) -> PeerId {
        self.wrapper().peer_id()
    }

    /// Creates a connection (delegates to wrapper).
    pub fn create_connection(&mut self) -> ConnectionId {
        self.wrapper().create_connection()
    }

    /// Push an event to the wrapper's inbox (delegates to wrapper).
    pub fn push_event(&mut self, event: HubEvent) {
        self.wrapper().push_event(event)
    }

    pub fn with_document<F, R>(&mut self, doc_id: &DocumentId, f: F) -> Result<R, DocumentError>
    where
        F: FnOnce(&mut Automerge) -> R,
    {
        self.wrapper().with_document(doc_id, f)
    }

    pub fn with_document_by_actor<F, R>(
        &mut self,
        actor_id: DocumentActorId,
        f: F,
    ) -> Result<R, DocumentError>
    where
        F: FnOnce(&mut Automerge) -> R,
    {
        self.wrapper().with_document_by_actor(actor_id, f)
    }

    pub fn broadcast(&mut self, actor: DocumentActorId, msg: Vec<u8>) {
        self.wrapper().broadcast(actor, msg);
    }

    pub fn pop_ephemera(&mut self, actor_id: DocumentActorId) -> Vec<Vec<u8>> {
        self.wrapper().pop_ephemera(actor_id)
    }

    fn wrapper(&mut self) -> &mut SamodWrapper {
        self.network
            .samods
            .get_mut(self.samod_id)
            .expect("Samod not found")
    }

    pub fn set_announce_policy(&mut self, policy: Box<dyn Fn(DocumentId, PeerId) -> bool>) {
        self.network
            .samods
            .get_mut(self.samod_id)
            .unwrap()
            .set_announce_policy(policy);
    }

    pub fn pop_doc_changed(&mut self, actor_id: DocumentActorId) -> Vec<DocumentChanged> {
        self.wrapper().pop_doc_changed(actor_id)
    }

    pub fn stop(&mut self) {
        self.wrapper().stop();
    }
}
