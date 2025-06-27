use std::sync::atomic::AtomicUsize;

static LAST_IO_TASK_ID: AtomicUsize = AtomicUsize::new(0);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct IoTaskId(usize);

impl IoTaskId {
    pub(crate) fn new() -> Self {
        let id = LAST_IO_TASK_ID.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        IoTaskId(id)
    }
}

impl From<usize> for IoTaskId {
    fn from(id: usize) -> Self {
        IoTaskId(id)
    }
}
