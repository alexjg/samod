use std::collections::HashSet;

use automerge::Automerge;

use crate::{DocumentId, StorageKey};

mod compaction_hash;
use compaction_hash::CompactionHash;

#[derive(Debug)]
pub(super) struct SaveState {
    last_saved_heads: Option<Vec<automerge::ChangeHash>>,
    on_disk: HashSet<StorageKey>,
    compaction: Option<Compaction>,
    deletions: HashSet<StorageKey>,
}

#[derive(Debug)]
struct Compaction {
    key: StorageKey,
    supercedes: HashSet<StorageKey>,
}

pub(crate) enum Job {
    Put { key: StorageKey, data: Vec<u8> },
    Delete(StorageKey),
}

pub(crate) enum JobComplete {
    Put(StorageKey),
    Delete(StorageKey),
}

impl SaveState {
    pub(super) fn new() -> Self {
        Self {
            last_saved_heads: None,
            on_disk: HashSet::new(),
            compaction: None,
            deletions: HashSet::new(),
        }
    }

    pub(super) fn add_on_disk<I: Iterator<Item = StorageKey>>(&mut self, contents: I) {
        self.on_disk.extend(contents);
    }

    pub(super) fn pop_new_jobs(&mut self, doc_id: &DocumentId, doc: &Automerge) -> Vec<Job> {
        let mut jobs = Vec::new();

        let new_changes = doc.get_changes(self.last_saved_heads.as_ref().unwrap_or(&Vec::new()));

        let eligible_for_compaction = new_changes.len() > 10 || self.on_disk.len() > 10;
        if self.compaction.is_none() && eligible_for_compaction {
            tracing::debug!(
                num_changes = new_changes.len(),
                num_on_disk = self.on_disk.len(),
                "compacting changes"
            );
            let hash = CompactionHash::from(doc.get_heads());
            let key = StorageKey::snapshot_path(doc_id.clone(), hash.to_string());
            self.compaction = Some(Compaction {
                key: key.clone(),
                supercedes: self.on_disk.clone(),
            });
            jobs.push(Job::Put {
                key,
                data: doc.save(),
            })
        } else {
            jobs.extend(new_changes.into_iter().map(|change| Job::Put {
                key: StorageKey::incremental_path(doc_id, change.hash()),
                data: change.raw_bytes().to_vec(),
            }));
        }

        jobs.extend(self.deletions.drain().map(Job::Delete));

        self.last_saved_heads = Some(doc.get_heads());

        jobs
    }

    pub(super) fn mark_job_complete(&mut self, completion: JobComplete) {
        match completion {
            JobComplete::Put(key) => {
                if let Some(compaction) = &mut self.compaction {
                    if compaction.key == key {
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
                }
                self.on_disk.insert(key);
            }
            JobComplete::Delete(key) => {
                // Probably this has already been removed in the handling of the compaction completion, but
                // we might as well be robust
                self.on_disk.remove(&key);
            }
        }
    }
}
