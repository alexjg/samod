#[derive(Clone)]
pub enum SyncMessage {
    /// Request for a document from a peer
    Request { data: Vec<u8> },
    /// Sync data containing document changes
    Sync { data: Vec<u8> },
    /// Notification that a peer doesn't have the document
    DocUnavailable,
}

impl std::fmt::Debug for SyncMessage {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SyncMessage::Request { data } => {
                write!(f, "SyncMessage::Request(data: {:?} bytes)", data.len())
            }
            SyncMessage::Sync { data } => {
                write!(f, "SyncMessage::Sync(data: {:?} bytes)", data.len())
            }
            SyncMessage::DocUnavailable => write!(f, "SyncMessage::DocUnavailable"),
        }
    }
}
