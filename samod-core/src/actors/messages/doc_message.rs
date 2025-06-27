use crate::ephemera::EphemeralMessage;

use super::SyncMessage;

#[derive(Clone, Debug)]
pub(crate) enum DocMessage {
    Sync(SyncMessage),
    Ephemeral(EphemeralMessage),
}
