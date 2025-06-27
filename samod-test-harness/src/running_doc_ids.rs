use samod_core::{DocumentActorId, DocumentId};

#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct RunningDocIds {
    pub doc_id: DocumentId,
    pub actor_id: DocumentActorId,
}
