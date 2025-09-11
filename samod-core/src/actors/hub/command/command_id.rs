use std::sync::atomic::AtomicUsize;

static LAST_COMMAND_ID: AtomicUsize = AtomicUsize::new(0);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct CommandId(usize);

impl CommandId {
    pub(crate) fn new() -> Self {
        let id = LAST_COMMAND_ID.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        CommandId(id)
    }
}

impl std::fmt::Display for CommandId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl From<CommandId> for u32 {
    fn from(id: CommandId) -> Self {
        id.0 as u32
    }
}

impl From<u32> for CommandId {
    fn from(id: u32) -> Self {
        CommandId(id as usize)
    }
}
