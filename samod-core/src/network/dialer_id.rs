use std::sync::atomic::{AtomicU32, Ordering};

static LAST_DIALER_ID: AtomicU32 = AtomicU32::new(0);

/// A unique identifier for a dialer in the samod-core system.
///
/// A dialer represents a persistent outgoing connection that automatically
/// reconnects with backoff. Each dialer has at most one active connection
/// at a time.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct DialerId(u32);

impl DialerId {
    pub(crate) fn new() -> Self {
        let id = LAST_DIALER_ID.fetch_add(1, Ordering::SeqCst);
        DialerId(id)
    }
}

impl std::fmt::Display for DialerId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl From<DialerId> for u32 {
    fn from(id: DialerId) -> Self {
        id.0
    }
}

impl From<u32> for DialerId {
    fn from(id: u32) -> Self {
        DialerId(id)
    }
}
