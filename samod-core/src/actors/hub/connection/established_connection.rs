use std::collections::HashMap;

use crate::{
    DocumentId, PeerId, UnixTimestamp,
    network::{PeerDocState, PeerMetadata},
};

/// State of an established connection after successful handshake.
#[derive(Debug, Clone)]
pub(crate) struct EstablishedConnection {
    /// Remote peer's ID
    pub(super) remote_peer_id: PeerId,
    /// Remote peer's metadata (storage ID, ephemeral flag)
    #[allow(dead_code)]
    pub(super) remote_metadata: Option<PeerMetadata>,
    /// Agreed protocol version
    #[allow(dead_code)]
    pub(super) protocol_version: String,
    /// When the connection was established
    #[allow(dead_code)]
    pub(super) established_at: UnixTimestamp,
    /// Documents this connection is actively syncing
    pub(super) document_subscriptions: HashMap<DocumentId, PeerDocState>,
}

impl EstablishedConnection {
    /// Get the remote peer ID.
    pub fn remote_peer_id(&self) -> &PeerId {
        &self.remote_peer_id
    }

    pub(crate) fn document_subscriptions(&self) -> &HashMap<DocumentId, PeerDocState> {
        &self.document_subscriptions
    }

    pub fn add_document_subscription(&mut self, document_id: DocumentId) {
        self.document_subscriptions
            .insert(document_id, PeerDocState::empty());
    }
}
