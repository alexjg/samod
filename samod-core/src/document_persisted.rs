use automerge::ChangeHash;

/// Emitted when document changes have been persisted to storage.
#[derive(Clone, Debug)]
pub struct DocumentPersisted {
    /// The heads that have been confirmed as persisted to storage.
    pub persisted_heads: Vec<ChangeHash>,
}
