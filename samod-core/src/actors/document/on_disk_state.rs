use std::collections::{HashMap, HashSet};

use automerge::Automerge;

use crate::{
    DocumentId, StorageKey,
    actors::document::DocActorResult,
    io::{IoTaskId, StorageResult},
};

mod compaction_hash;
pub use compaction_hash::CompactionHash;

/// When a document's incremental changes are rewritten as one snapshot
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CompactionPolicy {
    /// Compact once the incremental changes written since the last snapshot
    /// add up to more than this fraction of that snapshot's size. Must not be
    /// negative or NaN.
    pub incremental_fraction: f64,
    /// Don't compact on size until there are more than this many
    /// incremental files
    pub min_incrementals: usize,
    /// Compact once there are more than this many incremental files
    pub max_incrementals: usize,
}

impl Default for CompactionPolicy {
    fn default() -> Self {
        Self {
            incremental_fraction: 1.0,
            min_incrementals: 10,
            max_incrementals: 4096,
        }
    }
}

#[derive(Debug)]
pub(super) struct OnDiskState {
    policy: CompactionPolicy,
    last_saved_heads: Option<Vec<automerge::ChangeHash>>,
    on_disk: HashSet<StorageKey>,
    snapshot_bytes: usize,
    incrementals: usize,
    incremental_bytes: usize,
    compaction: Option<Compaction>,
    deletions: HashSet<StorageKey>,
    running_puts: HashMap<IoTaskId, StorageKey>,
    running_deletes: HashMap<IoTaskId, StorageKey>,
}

#[derive(Debug)]
struct Compaction {
    task: IoTaskId,
    supercedes: HashSet<StorageKey>,
}

impl OnDiskState {
    pub(super) fn new(policy: CompactionPolicy) -> Self {
        Self {
            policy,
            last_saved_heads: None,
            on_disk: HashSet::new(),
            snapshot_bytes: 0,
            incrementals: 0,
            incremental_bytes: 0,
            compaction: None,
            deletions: HashSet::new(),
            running_deletes: HashMap::new(),
            running_puts: HashMap::new(),
        }
    }

    pub(super) fn add_loaded(
        &mut self,
        snapshots: &HashMap<StorageKey, Vec<u8>>,
        incrementals: &HashMap<StorageKey, Vec<u8>>,
        heads: Vec<automerge::ChangeHash>,
    ) {
        self.snapshot_bytes = snapshots.values().map(Vec::len).max().unwrap_or(0);
        self.incrementals = incrementals.len();
        self.incremental_bytes = incrementals.values().map(Vec::len).sum();
        self.on_disk
            .extend(snapshots.keys().chain(incrementals.keys()).cloned());
        self.last_saved_heads = Some(heads);
    }

    pub(super) fn save_new_changes(
        &mut self,
        out: &mut DocActorResult,
        doc_id: &DocumentId,
        doc: &Automerge,
    ) {
        let new_changes = doc.get_changes(self.last_saved_heads.as_ref().unwrap_or(&Vec::new()));

        let incrementals = self.incrementals + new_changes.len();
        let incremental_bytes = self.incremental_bytes
            + new_changes
                .iter()
                .map(|c| c.raw_bytes().len())
                .sum::<usize>();
        let eligible_for_compaction = incrementals > self.policy.max_incrementals
            || (incrementals > self.policy.min_incrementals
                && incremental_bytes as f64
                    > self.snapshot_bytes as f64 * self.policy.incremental_fraction);
        if self.compaction.is_none() && eligible_for_compaction {
            tracing::debug!(
                num_changes = new_changes.len(),
                num_on_disk = self.on_disk.len(),
                incremental_bytes,
                snapshot_bytes = self.snapshot_bytes,
                "compacting changes"
            );
            let hash = CompactionHash::new(&doc.get_heads());
            let key = StorageKey::snapshot_path(doc_id, &hash);
            let snapshot = doc.save();
            self.snapshot_bytes = snapshot.len();
            self.incrementals = 0;
            self.incremental_bytes = 0;
            let put_id = out.put(key.clone(), snapshot);
            let mut supercedes = self.on_disk.clone();
            // Make sure that we don't delete the compaction we're producing
            // somehow One way this can happen is if a previous process got
            // killed after writing the compacted chunk but before deleting the
            // incremental changes.
            supercedes.remove(&key);
            self.compaction = Some(Compaction {
                task: put_id,
                supercedes,
            });
            self.running_puts.insert(put_id, key);
        } else {
            self.incrementals = incrementals;
            self.incremental_bytes = incremental_bytes;
            for change in new_changes {
                let key = StorageKey::incremental_path(doc_id, change.hash());
                let put_id = out.put(key.clone(), change.raw_bytes().to_vec());
                self.running_puts.insert(put_id, key);
            }
        }

        for deletion in self.deletions.drain() {
            let delete_id = out.delete(deletion.clone());
            self.running_deletes.insert(delete_id, deletion);
        }

        self.last_saved_heads = Some(doc.get_heads());
    }

    pub(super) fn is_flushed(&self) -> bool {
        self.running_puts.is_empty()
    }

    pub(super) fn has_task(&self, task_id: IoTaskId) -> bool {
        self.running_puts.contains_key(&task_id) || self.running_deletes.contains_key(&task_id)
    }

    pub(super) fn task_complete(&mut self, task_id: IoTaskId, result: StorageResult) {
        match result {
            StorageResult::Put => {
                if let Some(compaction_key) = self.running_puts.remove(&task_id) {
                    tracing::debug!(key=%compaction_key, "compaction put completed for key");
                    self.mark_put_complete(task_id, compaction_key);
                } else {
                    panic!("put complete for unknown task id: {:?}", task_id);
                }
            }
            StorageResult::Delete => {
                if let Some(deletion_key) = self.running_deletes.remove(&task_id) {
                    self.mark_delete_complete(deletion_key);
                } else {
                    panic!("delete complete for unknown task id: {:?}", task_id);
                }
            }
            _ => panic!("unexpected storage result"),
        }
    }

    fn mark_put_complete(&mut self, task_id: IoTaskId, key: StorageKey) {
        if let Some(compaction) = &mut self.compaction
            && compaction.task == task_id
        {
            tracing::debug!("compacted chunk saved, deleting superceded data");
            // We're marking these as deleted before we get confirmation that they
            // are deleted. This is okay, worst case we end up with some extra data
            // on disk which gets cleared up on next boot.
            for key in &compaction.supercedes {
                self.on_disk.remove(key);
            }
            self.deletions
                .extend(std::mem::take(&mut compaction.supercedes));
            self.compaction = None;
        }
        self.on_disk.insert(key);
    }

    fn mark_delete_complete(&mut self, key: StorageKey) {
        // Probably this has already been removed in the handling of the compaction completion, but
        // we might as well be robust
        self.on_disk.remove(&key);
    }
}
