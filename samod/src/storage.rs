//! The storage abstraction

use std::collections::HashMap;

pub use samod_core::StorageKey;

mod filesystem;
mod in_memory;
pub use in_memory::InMemoryStorage;

#[cfg(feature = "tokio")]
pub use filesystem::tokio::FilesystemStorage as TokioFilesystemStorage;

#[cfg(feature = "gio")]
pub use filesystem::gio::FilesystemStorage as GioFilesystemStorage;

/// The storage abstraction used by a [`Repo`](crate::Repo) to store document data
///
/// This trait is designed to be pretty general. It's effectively a key/value store
/// with range queries. In particular there are no assumptions about the ordering of
/// operations between calls to methods on storage and no assumption of exclusive
/// access to storage.
///
/// I.e. you can have multiple storage instances mutating the same shared storage,
/// `samod` is designed so that this will never lose data.
///
/// ## Storage Keys
///
/// Storage is a key/value store. The keys are effectively `Vec<String>`. This
/// matches things like filesystems, where each element in the `Vec<String>` is
/// a directory and the last item is a file. It can just as well be implemented
/// in other mediums. To make this easier `StorageKey` guarantees that none of
/// the components of the key contain a "/", this means you can use `"/"` to
/// join elements of the key when storing the key as a string.
pub trait Storage: Send + Clone + 'static {
    /// Load a specific key from storage
    fn load(&self, key: StorageKey) -> impl Future<Output = Option<Vec<u8>>> + Send;
    /// Load a range of keys from storage, all of which begin with `prefix`
    ///
    /// Note that you can use [`StorageKey::is_prefix_of`] to implement this
    /// in simple cases
    fn load_range(
        &self,
        prefix: StorageKey,
    ) -> impl Future<Output = HashMap<StorageKey, Vec<u8>>> + Send;
    /// Put a particular value into storage
    fn put(&self, key: StorageKey, data: Vec<u8>) -> impl Future<Output = ()> + Send;
    /// Delete a value from storage
    fn delete(&self, key: StorageKey) -> impl Future<Output = ()> + Send;
}
