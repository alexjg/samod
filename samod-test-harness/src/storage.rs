use std::collections::HashMap;

use samod_core::{
    StorageKey,
    io::{IoTaskId, StorageResult, StorageTask},
};

pub struct Storage {
    data: HashMap<StorageKey, Vec<u8>>,
    state: StorageState,
    completed_tasks: HashMap<IoTaskId, StorageResult>,
}

enum StorageState {
    Running,
    Paused {
        pending_tasks: HashMap<IoTaskId, StorageTask>,
    },
}

impl From<HashMap<StorageKey, Vec<u8>>> for Storage {
    fn from(map: HashMap<StorageKey, Vec<u8>>) -> Self {
        Storage {
            data: map,
            state: StorageState::Running,
            completed_tasks: HashMap::new(),
        }
    }
}

impl Storage {
    pub(crate) fn new() -> Self {
        Storage {
            data: HashMap::new(),
            state: StorageState::Running,
            completed_tasks: HashMap::new(),
        }
    }

    pub(crate) fn data(&self) -> &HashMap<StorageKey, Vec<u8>> {
        &self.data
    }

    pub(crate) fn pause(&mut self) {
        if let StorageState::Running = self.state {
            self.state = StorageState::Paused {
                pending_tasks: HashMap::new(),
            };
        }
    }

    pub(crate) fn resume(&mut self) {
        if let StorageState::Paused { pending_tasks } = &mut self.state {
            let tasks = std::mem::take(pending_tasks);
            self.state = StorageState::Running;
            for (task_id, task) in tasks {
                self.perform_task(task_id, task);
            }
        }
    }

    pub(crate) fn check_pending_task(&mut self, task_id: IoTaskId) -> Option<StorageResult> {
        self.completed_tasks.remove(&task_id)
    }

    pub(crate) fn handle_task(&mut self, task_id: IoTaskId, task: StorageTask) {
        match &mut self.state {
            StorageState::Running => self.perform_task(task_id, task),
            StorageState::Paused { pending_tasks } => {
                pending_tasks.insert(task_id, task);
            }
        }
    }

    /// Execute a storage task and return the result immediately.
    ///
    /// Used for Hub-level storage (e.g. sedimentree data) where the result
    /// is consumed synchronously in `execute_io_tasks`.
    pub(crate) fn execute_task(&mut self, task: &StorageTask) -> StorageResult {
        match task {
            StorageTask::Load { key } => StorageResult::Load {
                value: self.data.get(key).cloned(),
            },
            StorageTask::LoadRange { prefix } => {
                let values = self
                    .data
                    .iter()
                    .filter(|(k, _)| prefix.is_prefix_of(k))
                    .map(|(k, v)| (k.clone(), v.clone()))
                    .collect();
                StorageResult::LoadRange { values }
            }
            StorageTask::Put { key, value } => {
                self.data.insert(key.clone(), value.clone());
                StorageResult::Put
            }
            StorageTask::Delete { key } => {
                self.data.remove(key);
                StorageResult::Delete
            }
        }
    }

    fn perform_task(&mut self, task_id: IoTaskId, task: StorageTask) {
        let result = match task {
            StorageTask::Load { key } => StorageResult::Load {
                value: self.data.get(&key).cloned(),
            },
            StorageTask::LoadRange { prefix } => {
                let values = self
                    .data
                    .iter()
                    .filter(|(k, _)| prefix.is_prefix_of(k))
                    .map(|(k, v)| (k.clone(), v.clone()))
                    .collect();
                StorageResult::LoadRange { values }
            }
            StorageTask::Put { key, value } => {
                self.data.insert(key.clone(), value);
                StorageResult::Put
            }
            StorageTask::Delete { key } => {
                self.data.remove(&key);
                StorageResult::Delete
            }
        };
        self.completed_tasks.insert(task_id, result);
    }
}
