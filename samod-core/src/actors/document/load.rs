use std::collections::HashMap;

use crate::{
    DocumentId, StorageKey,
    io::{IoTask, IoTaskId, StorageResult, StorageTask},
};

#[derive(Debug)]
enum LoadState {
    Idle,
    /// Beginning a fresh load
    Initial,
    /// Both tasks dispatched, waiting for results
    WaitingForResults {
        snapshot_task_id: IoTaskId,
        incremental_task_id: IoTaskId,
        snapshots: Option<HashMap<StorageKey, Vec<u8>>>,
        incrementals: Option<HashMap<StorageKey, Vec<u8>>>,
    },
    /// Have both snapshots and incrementals, ready to complete
    Complete {
        snapshots: HashMap<StorageKey, Vec<u8>>,
        incrementals: HashMap<StorageKey, Vec<u8>>,
    },
}

#[derive(Debug)]
pub(crate) struct Load {
    doc_id: DocumentId,
    state: LoadState,
}

pub(crate) struct LoadComplete {
    pub(crate) snapshots: HashMap<StorageKey, Vec<u8>>,
    pub(crate) incrementals: HashMap<StorageKey, Vec<u8>>,
}

impl Load {
    /// Create a new Load operation for the given document ID
    pub(crate) fn new(doc_id: DocumentId) -> Self {
        Self {
            doc_id,
            state: LoadState::Idle,
        }
    }

    pub(crate) fn begin(&mut self) {
        if matches!(self.state, LoadState::Idle) {
            self.state = LoadState::Initial;
        }
    }

    pub(crate) fn has_task(&self, task_id: IoTaskId) -> bool {
        let LoadState::WaitingForResults {
            snapshot_task_id,
            incremental_task_id,
            snapshots: _,
            incrementals: _,
        } = &self.state
        else {
            return false;
        };
        (snapshot_task_id == &task_id) || (incremental_task_id == &task_id)
    }

    /// Progress the load operation, returning any IO tasks that need to be dispatched
    pub(crate) fn step(&mut self) -> Vec<IoTask<StorageTask>> {
        match &self.state {
            LoadState::Idle => Vec::new(),
            LoadState::Initial => {
                let snapshot_task = IoTask::new(StorageTask::LoadRange {
                    prefix: StorageKey::snapshot_prefix(&self.doc_id),
                });

                let incremental_task = IoTask::new(StorageTask::LoadRange {
                    prefix: StorageKey::incremental_prefix(&self.doc_id),
                });

                let snapshot_task_id = snapshot_task.task_id;
                let incremental_task_id = incremental_task.task_id;

                self.state = LoadState::WaitingForResults {
                    snapshot_task_id,
                    incremental_task_id,
                    snapshots: None,
                    incrementals: None,
                };

                vec![snapshot_task, incremental_task]
            }
            LoadState::WaitingForResults { .. } | LoadState::Complete { .. } => Vec::new(),
        }
    }

    pub fn take_complete(&mut self) -> Option<LoadComplete> {
        if let LoadState::Complete {
            snapshots,
            incrementals,
            ..
        } = &mut self.state
        {
            let result = LoadComplete {
                snapshots: std::mem::take(snapshots),
                incrementals: std::mem::take(incrementals),
            };
            self.state = LoadState::Idle;
            Some(result)
        } else {
            None
        }
    }

    /// Handle the result of an IO operation
    pub fn handle_result(&mut self, task_id: IoTaskId, result: StorageResult) {
        match &mut self.state {
            LoadState::WaitingForResults {
                snapshot_task_id,
                incremental_task_id,
                snapshots,
                incrementals,
            } => {
                match result {
                    StorageResult::LoadRange { values } => {
                        if task_id == *snapshot_task_id {
                            *snapshots = Some(values);
                        } else if task_id == *incremental_task_id {
                            *incrementals = Some(values);
                        } else {
                            panic!("unknown task ID in load");
                        }

                        // Check if both results are now available
                        if let (Some(snapshots), Some(incrementals)) = (snapshots, incrementals) {
                            self.state = LoadState::Complete {
                                snapshots: snapshots.clone(),
                                incrementals: incrementals.clone(),
                            };
                        }
                    }
                    _ => panic!("unexpected storage result in load"),
                }
            }
            _ => panic!("unexpected io complete"),
        }
    }
}
