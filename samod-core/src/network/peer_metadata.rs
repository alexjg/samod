use super::wire_protocol;

/// Metadata about a peer from the handshake.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PeerMetadata {
    /// Whether the peer expects to connect again with this storage ID
    pub is_ephemeral: bool,
}

impl PeerMetadata {
    /// Convert to wire protocol PeerMetadata for sending over network
    pub(crate) fn to_wire(
        &self,
        storage_id: Option<crate::StorageId>,
    ) -> wire_protocol::PeerMetadata {
        wire_protocol::PeerMetadata {
            storage_id,
            is_ephemeral: self.is_ephemeral,
        }
    }

    /// Convert from wire protocol PeerMetadata when receiving from network
    pub(crate) fn from_wire(wire: wire_protocol::PeerMetadata) -> Self {
        Self {
            is_ephemeral: wire.is_ephemeral,
        }
    }
}
