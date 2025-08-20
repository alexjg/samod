use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};

use samod_core::StorageKey;

use crate::storage::Storage;

/// A [`Storage`] implementation which stores all data in memory
#[derive(Clone)]
pub struct InMemoryStorage(Arc<Mutex<HashMap<StorageKey, Vec<u8>>>>);

impl InMemoryStorage {
    pub fn new() -> Self {
        Self(Arc::new(Mutex::new(HashMap::new())))
    }
}

impl Default for InMemoryStorage {
    fn default() -> Self {
        Self::new()
    }
}

impl Storage for InMemoryStorage {
    #[tracing::instrument(skip(self), level = "trace", ret)]
    fn load(&self, key: StorageKey) -> impl Future<Output = Option<Vec<u8>>> + Send {
        futures::future::ready(self.0.lock().unwrap().get(&key).cloned())
    }

    #[tracing::instrument(skip(self), level = "trace", ret)]
    fn load_range(
        &self,
        prefix: StorageKey,
    ) -> impl Future<Output = HashMap<StorageKey, Vec<u8>>> + Send {
        futures::future::ready(
            self.0
                .lock()
                .unwrap()
                .iter()
                .filter(|(k, _)| prefix.is_prefix_of(k))
                .map(|(k, v)| (k.clone(), v.clone()))
                .collect(),
        )
    }

    #[tracing::instrument(skip(self, data), level = "trace")]
    fn put(&self, key: StorageKey, data: Vec<u8>) -> impl Future<Output = ()> + Send {
        self.0.lock().unwrap().insert(key, data);
        futures::future::ready(())
    }

    #[tracing::instrument(skip(self), level = "trace")]
    fn delete(&self, key: StorageKey) -> impl Future<Output = ()> + Send {
        self.0.lock().unwrap().remove(&key);
        futures::future::ready(())
    }
}
