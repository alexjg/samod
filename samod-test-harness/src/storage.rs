use std::collections::HashMap;

use samod_core::{
    StorageKey,
    io::{StorageResult, StorageTask},
};

pub struct Storage(pub(crate) HashMap<StorageKey, Vec<u8>>);

impl From<HashMap<StorageKey, Vec<u8>>> for Storage {
    fn from(map: HashMap<StorageKey, Vec<u8>>) -> Self {
        Storage(map)
    }
}

impl Storage {
    pub(crate) fn new() -> Self {
        Storage(HashMap::new())
    }

    pub(crate) fn handle_task(&mut self, task: StorageTask) -> StorageResult {
        match task {
            StorageTask::Load { key } => StorageResult::Load {
                value: self.0.get(&key).cloned(),
            },
            StorageTask::LoadRange { prefix } => {
                let values = self
                    .0
                    .iter()
                    .filter(|(k, _)| prefix.is_prefix_of(k))
                    .map(|(k, v)| (k.clone(), v.clone()))
                    .collect();
                StorageResult::LoadRange { values }
            }
            StorageTask::Put { key, value } => {
                self.0.insert(key.clone(), value);
                StorageResult::Put
            }
            StorageTask::Delete { key } => {
                self.0.remove(&key);
                StorageResult::Delete
            }
        }
    }
}
