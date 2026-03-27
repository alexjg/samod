use crate::{ConnectionId, io::StorageTask};

#[derive(Debug, Clone)]
pub enum HubIoAction {
    Send {
        connection_id: ConnectionId,
        msg: Vec<u8>,
    },

    Disconnect {
        connection_id: ConnectionId,
    },

    /// A storage operation issued by the Hub (e.g., for sedimentree data).
    Storage(StorageTask),

    /// Sign the given payload bytes with the configured signing key.
    ///
    /// Used by the subduction handshake and commit signing.
    /// The result should be fed back via `HubIoResult::Sign`.
    #[cfg(feature = "subduction")]
    Sign {
        payload_bytes: Vec<u8>,
    },
}
