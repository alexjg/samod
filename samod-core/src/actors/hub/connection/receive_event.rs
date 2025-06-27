use crate::{DocumentId, PeerId, actors::messages::SyncMessage, ephemera::EphemeralSessionId};

pub(crate) enum ReceiveEvent {
    HandshakeComplete {
        remote_peer_id: PeerId,
    },
    SyncMessage {
        doc_id: DocumentId,
        #[allow(dead_code)]
        sender_id: PeerId,
        target_id: PeerId,
        msg: SyncMessage,
    },
    EphemeralMessage {
        doc_id: DocumentId,
        sender_id: PeerId,
        target_id: PeerId,
        count: u64,
        session_id: EphemeralSessionId,
        msg: Vec<u8>,
    },
}
