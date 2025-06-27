use std::sync::atomic::AtomicUsize;

static LAST_SAMOD_ID: AtomicUsize = AtomicUsize::new(0);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SamodId(usize);

impl SamodId {
    pub(crate) fn new() -> Self {
        let id = LAST_SAMOD_ID.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        SamodId(id)
    }
}
