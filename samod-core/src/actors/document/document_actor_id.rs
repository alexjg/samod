use std::sync::atomic::{AtomicU32, Ordering};

static LAST_DOCUMENT_ACTOR_ID: AtomicU32 = AtomicU32::new(0);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct DocumentActorId(u32);

impl Default for DocumentActorId {
    fn default() -> Self {
        Self::new()
    }
}

impl DocumentActorId {
    pub fn new() -> Self {
        let id = LAST_DOCUMENT_ACTOR_ID.fetch_add(1, Ordering::SeqCst);
        DocumentActorId(id)
    }
}

impl std::fmt::Display for DocumentActorId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "actor:{}", self.0)
    }
}

impl From<DocumentActorId> for u32 {
    fn from(id: DocumentActorId) -> Self {
        id.0
    }
}

impl From<u32> for DocumentActorId {
    fn from(id: u32) -> Self {
        DocumentActorId(id)
    }
}
