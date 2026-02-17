use std::sync::atomic::{AtomicU32, Ordering};

static LAST_LISTENER_ID: AtomicU32 = AtomicU32::new(0);

/// A unique identifier for a listener in the samod-core system.
///
/// A listener represents a passive endpoint that accepts inbound connections.
/// Each listener can hold zero or more active connections simultaneously.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ListenerId(u32);

impl ListenerId {
    pub(crate) fn new() -> Self {
        let id = LAST_LISTENER_ID.fetch_add(1, Ordering::SeqCst);
        ListenerId(id)
    }
}

impl std::fmt::Display for ListenerId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl From<ListenerId> for u32 {
    fn from(id: ListenerId) -> Self {
        id.0
    }
}

impl From<u32> for ListenerId {
    fn from(id: u32) -> Self {
        ListenerId(id)
    }
}
