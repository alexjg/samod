use crate::{PeerId, ephemera::EphemeralSessionId};

#[derive(Clone, Debug)]
pub(crate) struct EphemeralMessage {
    pub(crate) sender_id: PeerId,
    pub(crate) session_id: EphemeralSessionId,
    pub(crate) count: u64,
    pub(crate) data: Vec<u8>,
}
