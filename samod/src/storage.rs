use std::collections::HashMap;

pub use samod_core::StorageKey;

mod filesystem;
mod in_memory;
pub use in_memory::InMemoryStorage;

#[cfg(feature = "tokio")]
pub use filesystem::tokio::FilesystemStorage as TokioFilesystemStorage;

#[cfg(feature = "gio")]
pub use filesystem::gio::FilesystemStorage as GioFilesystemStorage;

pub trait Storage: Send + Clone + 'static {
    fn load(&self, key: StorageKey) -> impl Future<Output = Option<Vec<u8>>> + Send;
    fn load_range(
        &self,
        prefix: StorageKey,
    ) -> impl Future<Output = HashMap<StorageKey, Vec<u8>>> + Send;
    fn put(&self, key: StorageKey, data: Vec<u8>) -> impl Future<Output = ()> + Send;
    fn delete(&self, key: StorageKey) -> impl Future<Output = ()> + Send;
}
