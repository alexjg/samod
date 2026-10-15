use std::{collections::HashMap, sync::Arc};

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
    ///
    /// Behind an `Arc` so that `ConnectionInfo` snapshots, which are emitted
    /// after most hub events, don't have to copy the whole map. Mutate via
    /// `Arc::make_mut`, which only copies if an old snapshot is still alive.
    pub(super) document_subscriptions: Arc<HashMap<DocumentId, PeerDocState>>,
}

impl EstablishedConnection {
    /// Get the remote peer ID.
    pub fn remote_peer_id(&self) -> &PeerId {
        &self.remote_peer_id
    }

    pub(crate) fn document_subscriptions(&self) -> &Arc<HashMap<DocumentId, PeerDocState>> {
        &self.document_subscriptions
    }

    pub fn add_document_subscription(&mut self, document_id: DocumentId) {
        Arc::make_mut(&mut self.document_subscriptions).insert(document_id, PeerDocState::empty());
    }

    pub(super) fn update_document_state(
        &mut self,
        document_id: &DocumentId,
        state: PeerDocState,
    ) -> bool {
        if !self.document_subscriptions.contains_key(document_id) {
            return false;
        }
        if let Some(doc_state) =
            Arc::make_mut(&mut self.document_subscriptions).get_mut(document_id)
        {
            *doc_state = state;
        }
        true
    }
}
