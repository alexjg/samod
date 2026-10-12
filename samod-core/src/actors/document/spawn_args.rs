use std::collections::HashMap;

use crate::{
    CompactionPolicy, ConnectionId, DocumentActorId, PeerId, actors::messages::DocMessage,
};

#[derive(Clone)]
pub struct SpawnArgs {
    pub(crate) local_peer_id: PeerId,
    pub(crate) actor_id: DocumentActorId,
    pub(crate) document_id: crate::DocumentId,
    pub(crate) initial_content: Option<automerge::Automerge>,
    pub(crate) initial_connections: HashMap<ConnectionId, (PeerId, Option<DocMessage>)>,
    pub(crate) compaction_policy: CompactionPolicy,
}

impl std::fmt::Debug for SpawnArgs {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SpawnArgs")
            .field("actor_id", &self.actor_id)
            .field("local_peer_id", &self.local_peer_id)
            .field("document_id", &self.document_id)
            .field(
                "initial_content",
                &self.initial_content.as_ref().map(|_| "<Automerge>"),
            )
            .field("initial_connections", &self.initial_connections)
            .field("compaction_policy", &self.compaction_policy)
            .finish()
    }
}

impl SpawnArgs {
    pub fn actor_id(&self) -> DocumentActorId {
        self.actor_id
    }

    pub fn document_id(&self) -> &crate::DocumentId {
        &self.document_id
    }
}
