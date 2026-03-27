use crate::io::StorageResult;

#[derive(Debug, Clone)]
pub enum HubIoResult {
    Send,
    Disconnect,
    Storage(StorageResult),
    /// Result of a signing operation.
    #[cfg(feature = "subduction")]
    Sign {
        signature: ed25519_dalek::Signature,
    },
}
