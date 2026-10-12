use crate::{PeerId, io::StorageTask};

#[derive(Debug)]
pub enum DocumentIoTask {
    Storage(StorageTask),
    /// Resolve sharing permissions before any document sync is processed.
    CheckSharePolicy {
        peer_id: PeerId,
    },
}
