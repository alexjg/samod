use std::collections::{HashMap, HashSet};

use automerge::Automerge;

use crate::{
    DocumentId, StorageKey,
    actors::document::DocActorResult,
    io::{IoTaskId, StorageResult},
};

#[cfg(feature = "subduction")]
use sedimentree_core::crypto::digest::Digest;

mod compaction_hash;
pub use compaction_hash::CompactionHash;

#[derive(Debug)]
pub(super) struct OnDiskState {
    last_saved_heads: Option<Vec<automerge::ChangeHash>>,
    on_disk: HashSet<StorageKey>,
    compaction: Option<Compaction>,
    deletions: HashSet<StorageKey>,
    running_puts: HashMap<IoTaskId, StorageKey>,
    running_deletes: HashMap<IoTaskId, StorageKey>,
    /// Digests of fragments and loose commits already sent to subduction.
    #[cfg(feature = "subduction")]
    sent_fragment_digests: HashSet<Digest<sedimentree_core::fragment::Fragment>>,
    #[cfg(feature = "subduction")]
    sent_commit_digests: HashSet<Digest<sedimentree_core::loose_commit::LooseCommit>>,
}

#[derive(Debug)]
struct Compaction {
    task: IoTaskId,
    supercedes: HashSet<StorageKey>,
}

impl OnDiskState {
    pub(super) fn new() -> Self {
        Self {
            last_saved_heads: None,
            on_disk: HashSet::new(),
            compaction: None,
            deletions: HashSet::new(),
            running_deletes: HashMap::new(),
            running_puts: HashMap::new(),
            #[cfg(feature = "subduction")]
            sent_fragment_digests: HashSet::new(),
            #[cfg(feature = "subduction")]
            sent_commit_digests: HashSet::new(),
        }
    }

    /// Add keys that we know are now on disk
    pub(super) fn add_keys<I: Iterator<Item = StorageKey>>(&mut self, contents: I) {
        self.on_disk.extend(contents);
    }

    pub(super) fn save_new_changes(
        &mut self,
        out: &mut DocActorResult,
        doc_id: &DocumentId,
        doc: &Automerge,
    ) {
        let new_changes = doc.get_changes(self.last_saved_heads.as_ref().unwrap_or(&Vec::new()));

        let eligible_for_compaction = new_changes.len() > 10 || self.on_disk.len() > 10;
        if self.compaction.is_none() && eligible_for_compaction {
            tracing::debug!(
                num_changes = new_changes.len(),
                num_on_disk = self.on_disk.len(),
                "compacting changes"
            );
            let hash = CompactionHash::new(&doc.get_heads());
            let key = StorageKey::snapshot_path(doc_id, &hash);
            let put_id = out.put(key.clone(), doc.save());
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

        // Run sedimentree ingest and send new fragments/commits to the hub.
        #[cfg(feature = "subduction")]
        self.send_sedimentree_data(out, doc_id, doc);

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

    #[cfg(feature = "subduction")]
    fn send_sedimentree_data(
        &mut self,
        out: &mut DocActorResult,
        doc_id: &DocumentId,
        doc: &Automerge,
    ) {
        let sed_id = doc_id.to_sedimentree_id();
        let ingest_result = match automerge_sedimentree::ingest::ingest_automerge(doc, sed_id) {
            Ok(result) => result,
            Err(e) => {
                tracing::warn!(err=?e, "sedimentree ingest failed");
                return;
            }
        };

        // Diff against previously-sent state: only send new items.
        let mut new_fragments = Vec::new();
        let mut new_loose_commits = Vec::new();

        use sedimentree_core::blob::has_meta::HasBlobMeta;

        for (fragment_digest, fragment) in ingest_result.sedimentree.fragment_entries() {
            if self.sent_fragment_digests.insert(*fragment_digest) {
                if let Some(blob) = ingest_result
                    .blobs
                    .iter()
                    .find(|b| b.meta().digest() == fragment.blob_meta().digest())
                {
                    new_fragments.push((fragment.clone(), blob.clone()));
                }
            }
        }

        for (commit_digest, commit) in ingest_result.sedimentree.commit_entries() {
            if self.sent_commit_digests.insert(*commit_digest) {
                if let Some(blob) = ingest_result
                    .blobs
                    .iter()
                    .find(|b| b.meta().digest() == commit.blob_meta().digest())
                {
                    new_loose_commits.push((commit.clone(), blob.clone()));
                }
            }
        }

        if !new_fragments.is_empty() || !new_loose_commits.is_empty() {
            tracing::debug!(
                fragments = new_fragments.len(),
                loose_commits = new_loose_commits.len(),
                "sending sedimentree data to hub"
            );
            out.outgoing_messages.push(crate::actors::messages::DocToHubMsg(
                crate::actors::messages::DocToHubMsgPayload::NewSedimentreeData {
                    document_id: doc_id.clone(),
                    fragments: new_fragments,
                    loose_commits: new_loose_commits,
                },
            ));
        }
    }
}
