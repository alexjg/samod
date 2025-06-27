use crate::{PeerId, io::StorageTask};

#[derive(Debug)]
pub enum DocumentIoTask {
    Storage(StorageTask),
    CheckAnnouncePolicy { peer_id: PeerId },
}
