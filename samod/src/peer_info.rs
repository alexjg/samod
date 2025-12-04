use crate::{PeerId, StorageId};

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct PeerInfo {
    pub peer_id: PeerId,
    pub storage_id: Option<StorageId>,
}

impl From<samod_core::network::PeerInfo> for PeerInfo {
    fn from(peer_info: samod_core::network::PeerInfo) -> Self {
        PeerInfo {
            peer_id: peer_info.peer_id,
            storage_id: peer_info.metadata.and_then(|d| d.storage_id),
        }
    }
}
