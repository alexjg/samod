use crate::storage_key::StorageKey;

/// Represents storage operations that can be performed by both the main Samod instance
/// and document actors.
///
/// `StorageTask` defines the common interface for all storage operations:
/// - Single value operations (Load, Put, Delete)
/// - Bulk operations (LoadRange)
///
/// This enum is used in two contexts:
/// 1. As part of `IoAction::Storage(StorageTask)` for the main Samod instance
/// 2. As part of `DocumentIoTask` for document actors
///
/// ## Storage Model
///
/// Storage operations work with a simple key-value model:
/// - Keys are represented by `StorageKey`
/// - Values are arbitrary byte arrays
/// - Range queries are supported via prefix matching
#[derive(Debug, Clone)]
pub enum StorageTask {
    /// Load a single value from storage by its key.
    ///
    /// This operation should retrieve the value associated with the given key
    /// from persistent storage. If the key doesn't exist, the operation should
    /// complete successfully with a `None` result.
    ///
    /// # Fields
    ///
    /// * `key` - The storage key to look up
    Load { key: StorageKey },

    /// Load all key-value pairs that have keys starting with the given prefix.
    ///
    /// This operation performs a range query over the storage, returning all
    /// entries whose keys begin with the specified prefix. This is used for
    /// efficient bulk operations and queries over related data.
    ///
    /// # Fields
    ///
    /// * `prefix` - The key prefix to match against
    LoadRange { prefix: StorageKey },

    /// Store a key-value pair in persistent storage.
    ///
    /// This operation should persist the given key-value pair to storage,
    /// replacing any existing value for the same key.
    ///
    /// # Fields
    ///
    /// * `key` - The storage key
    /// * `value` - The data to store
    Put { key: StorageKey, value: Vec<u8> },

    /// Remove a key-value pair from persistent storage.
    ///
    /// This operation should remove the entry for the given key from storage.
    /// If the key doesn't exist, the operation should complete successfully
    /// as a no-op.
    ///
    /// # Fields
    ///
    /// * `key` - The storage key to remove
    Delete { key: StorageKey },
}
