use crate::{DocumentActorId, DocumentId, actors::document::DocumentStatus};

#[derive(Debug, Clone)]
pub(crate) struct ActorInfo {
    pub(crate) actor_id: DocumentActorId,
    pub(crate) document_id: DocumentId,
    pub(crate) status: DocumentStatus,
}

impl ActorInfo {
    pub fn new_with_id(actor_id: DocumentActorId, document_id: DocumentId) -> Self {
        Self {
            actor_id,
            document_id,
            status: DocumentStatus::Spawned,
        }
    }
}
