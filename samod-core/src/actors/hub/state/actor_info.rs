use crate::{DocumentActorId, DocumentId, doc_search::DocSearchPhase};

#[derive(Debug, Clone)]
pub(crate) struct ActorInfo {
    pub(crate) actor_id: DocumentActorId,
    pub(crate) document_id: DocumentId,
    pub(crate) search_phase: DocSearchPhase,
}

impl ActorInfo {
    pub fn new_with_id(actor_id: DocumentActorId, document_id: DocumentId) -> Self {
        Self {
            actor_id,
            document_id,
            search_phase: DocSearchPhase::Loading,
        }
    }
}
