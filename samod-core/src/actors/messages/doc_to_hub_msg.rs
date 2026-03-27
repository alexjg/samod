use std::collections::HashMap;

use crate::{
    ConnectionId, DocumentId, actors::document::DocumentStatus, ephemera::EphemeralMessage,
    network::PeerDocState,
};

use super::SyncMessage;

/// Messages sent from document actors to Samod.
#[derive(Debug, Clone)]
pub struct DocToHubMsg(pub(crate) DocToHubMsgPayload);

#[derive(Debug, Clone)]
pub(crate) enum DocToHubMsgPayload {
    DocumentStatusChanged {
        new_status: DocumentStatus,
    },

    PeerStatesChanged {
        new_states: HashMap<ConnectionId, PeerDocState>,
    },

    SendSyncMessage {
        connection_id: ConnectionId,
        document_id: DocumentId,
        message: SyncMessage,
    },

    Broadcast {
        connections: Vec<ConnectionId>,
        msg: Broadcast,
    },

    Terminated,

    /// New local changes that need to be propagated via subduction.
    ///
    /// Contains structured change information so the Hub can build sedimentree
    /// commits and blobs, sign them, and push to subscribed subduction peers.
    #[cfg(feature = "subduction")]
    NewChangesForSubduction {
        document_id: DocumentId,
        changes: Vec<SubductionChangeInfo>,
    },
}

/// Information about a single automerge change for subduction propagation.
#[cfg(feature = "subduction")]
#[derive(Debug, Clone)]
pub struct SubductionChangeInfo {
    /// The hash of this change (becomes the commit digest in sedimentree).
    pub hash: automerge::ChangeHash,
    /// Parent change hashes (becomes the parents in LooseCommit).
    pub deps: Vec<automerge::ChangeHash>,
    /// Raw change bytes (becomes the blob in sedimentree).
    pub raw_bytes: Vec<u8>,
}

#[derive(Debug, Clone)]
pub enum Broadcast {
    New { msg: Vec<u8> },
    Gossip { msg: EphemeralMessage },
}
