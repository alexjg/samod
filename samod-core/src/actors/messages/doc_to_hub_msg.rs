use std::collections::HashMap;

use crate::{
    ConnectionId, DocumentId, doc_search::DocSearchPhase, ephemera::EphemeralMessage,
    network::PeerDocState,
};

use super::SyncMessage;

/// Messages sent from document actors to Samod.
#[derive(Debug, Clone)]
pub struct DocToHubMsg(pub(crate) DocToHubMsgPayload);

#[derive(Debug, Clone)]
pub(crate) enum DocToHubMsgPayload {
    DocSearchChanged(DocSearchPhase),

    PeerStatesChanged(HashMap<ConnectionId, PeerDocState>),

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
}

#[derive(Debug, Clone)]
pub enum Broadcast {
    New { msg: Vec<u8> },
    Gossip { msg: EphemeralMessage },
}
