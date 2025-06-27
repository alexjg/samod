use crate::{PeerId, network::PeerMetadata};

/// Information about a connected peer after successful handshake.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PeerInfo {
    pub peer_id: PeerId,
    pub metadata: Option<PeerMetadata>,
    pub protocol_version: String,
}
