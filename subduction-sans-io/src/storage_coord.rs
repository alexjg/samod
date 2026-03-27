//! Storage coordinator: manages multi-step KV operations for batch sync.
//!
//! The coordinator translates abstract `BatchSyncIo` requests into concrete
//! `KvOp` operations, tracks pending multi-step operations, and reassembles
//! results into `BatchSyncIoResult` when all constituent tasks complete.

use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};

use sedimentree_core::{
    blob::Blob,
    crypto::digest::Digest,
    fragment::Fragment,
    id::SedimentreeId,
    loose_commit::LooseCommit,
};
use subduction_crypto::signed::Signed;

use crate::{
    batch_sync::{BatchSyncIo, BatchSyncIoResult, BatchSyncSession},
    storage::{
        self, KvOp, KvResult, StorageKey,
        assemble_commit_pairs, assemble_fragment_pairs,
        reassemble_sedimentree_from_range,
        sedimentree_id_prefix, sedimentree_commit_path, sedimentree_fragment_path,
        sedimentree_blob_path,
    },
};

// ---------------------------------------------------------------------------
// OpId
// ---------------------------------------------------------------------------

static NEXT_OP_ID: AtomicUsize = AtomicUsize::new(0);

/// Opaque identifier for a pending storage operation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct OpId(usize);

impl OpId {
    pub fn new() -> Self {
        Self(NEXT_OP_ID.fetch_add(1, Ordering::SeqCst))
    }
}

// ---------------------------------------------------------------------------
// Public types
// ---------------------------------------------------------------------------

/// A storage operation the caller must execute, tagged with an OpId.
#[derive(Debug, Clone)]
pub struct IssuedOp {
    pub op_id: OpId,
    pub operation: KvOp,
}

/// A completed logical storage operation.
#[derive(Debug)]
pub struct StorageComplete {
    pub session: BatchSyncSession,
    pub result: BatchSyncIoResult,
}

// ---------------------------------------------------------------------------
// Pending operation state
// ---------------------------------------------------------------------------

#[derive(Debug)]
enum PendingOp {
    /// Single LoadRange on the full sedimentree prefix.
    LoadSedimentree {
        sed_id: SedimentreeId,
        session: BatchSyncSession,
    },
    /// Multiple Put tasks. When all complete → PutComplete.
    PutData {
        remaining: usize,
        session: BatchSyncSession,
    },
    /// Phase 1: loading signed commit bytes by digest.
    LoadCommitBlobsPhase1 {
        sed_id: SedimentreeId,
        /// Maps OpId sub-task → commit Digest.
        task_digests: HashMap<OpId, Digest<LooseCommit>>,
        loaded: HashMap<Digest<LooseCommit>, Vec<u8>>,
        remaining: usize,
        session: BatchSyncSession,
    },
    /// Phase 2: loading blob bytes (after deserializing commits from Phase 1).
    LoadCommitBlobsPhase2 {
        signed_commits: HashMap<Digest<LooseCommit>, Vec<u8>>,
        task_digests: HashMap<OpId, Digest<LooseCommit>>,
        loaded_blobs: HashMap<Digest<LooseCommit>, Vec<u8>>,
        remaining: usize,
        session: BatchSyncSession,
    },
    /// Phase 1 for fragments.
    LoadFragmentBlobsPhase1 {
        sed_id: SedimentreeId,
        task_digests: HashMap<OpId, Digest<Fragment>>,
        loaded: HashMap<Digest<Fragment>, Vec<u8>>,
        remaining: usize,
        session: BatchSyncSession,
    },
    /// Phase 2 for fragments.
    LoadFragmentBlobsPhase2 {
        signed_fragments: HashMap<Digest<Fragment>, Vec<u8>>,
        task_digests: HashMap<OpId, Digest<Fragment>>,
        loaded_blobs: HashMap<Digest<Fragment>, Vec<u8>>,
        remaining: usize,
        session: BatchSyncSession,
    },
}

// ---------------------------------------------------------------------------
// StorageCoordinator
// ---------------------------------------------------------------------------

/// Coordinates multi-step KV operations for batch sync sessions.
///
/// The caller issues `BatchSyncIo` operations via [`issue`](Self::issue),
/// executes the returned `KvOp`s against their storage backend, and feeds
/// results back via [`handle_result`](Self::handle_result). When a logical
/// operation completes, it returns a `StorageComplete` with the session
/// and result.
#[derive(Debug, Default)]
pub struct StorageCoordinator {
    pending: HashMap<OpId, PendingOp>,
    /// Maps sub-task OpId → parent OpId for multi-task operations.
    subtask_to_parent: HashMap<OpId, OpId>,
}

impl StorageCoordinator {
    pub fn new() -> Self {
        Self::default()
    }

    /// Issue storage operations for a `BatchSyncIo` request.
    ///
    /// Returns operations the caller must execute. Each operation has an
    /// `OpId` that must be passed back with the result via `handle_result`.
    pub fn issue(
        &mut self,
        session: BatchSyncSession,
        io: BatchSyncIo,
    ) -> Vec<IssuedOp> {
        let sed_id = session.sedimentree_id();
        match io {
            BatchSyncIo::LoadSedimentree { id } => {
                let op_id = OpId::new();
                self.pending.insert(op_id, PendingOp::LoadSedimentree {
                    sed_id: id, session,
                });
                vec![IssuedOp {
                    op_id,
                    operation: KvOp::LoadRange {
                        prefix: sedimentree_id_prefix(&id),
                    },
                }]
            }
            BatchSyncIo::PutCommits { commits, .. } => {
                self.issue_puts(sed_id, session, storage::commits_to_kv_ops(&sed_id, &commits))
            }
            BatchSyncIo::PutFragments { fragments, .. } => {
                self.issue_puts(sed_id, session, storage::fragments_to_kv_ops(&sed_id, &fragments))
            }
            BatchSyncIo::LoadCommitBlobs { digests, .. } => {
                let parent_id = OpId::new();
                let mut ops = Vec::with_capacity(digests.len());
                let mut task_digests = HashMap::new();

                for digest in &digests {
                    let sub_id = OpId::new();
                    self.subtask_to_parent.insert(sub_id, parent_id);
                    task_digests.insert(sub_id, *digest);
                    ops.push(IssuedOp {
                        op_id: sub_id,
                        operation: KvOp::Load {
                            key: sedimentree_commit_path(&sed_id, digest),
                        },
                    });
                }

                self.pending.insert(parent_id, PendingOp::LoadCommitBlobsPhase1 {
                    sed_id,
                    task_digests,
                    loaded: HashMap::new(),
                    remaining: digests.len(),
                    session,
                });
                ops
            }
            BatchSyncIo::LoadFragmentBlobs { digests, .. } => {
                let parent_id = OpId::new();
                let mut ops = Vec::with_capacity(digests.len());
                let mut task_digests = HashMap::new();

                for digest in &digests {
                    let sub_id = OpId::new();
                    self.subtask_to_parent.insert(sub_id, parent_id);
                    task_digests.insert(sub_id, *digest);
                    ops.push(IssuedOp {
                        op_id: sub_id,
                        operation: KvOp::Load {
                            key: sedimentree_fragment_path(&sed_id, digest),
                        },
                    });
                }

                self.pending.insert(parent_id, PendingOp::LoadFragmentBlobsPhase1 {
                    sed_id,
                    task_digests,
                    loaded: HashMap::new(),
                    remaining: digests.len(),
                    session,
                });
                ops
            }
        }
    }

    /// Issue fire-and-forget storage operations (no result tracking).
    pub fn issue_fire_and_forget(
        &self,
        sed_id: &SedimentreeId,
        io: BatchSyncIo,
    ) -> Vec<KvOp> {
        match io {
            BatchSyncIo::PutCommits { commits, .. } => {
                storage::commits_to_kv_ops(sed_id, &commits)
            }
            BatchSyncIo::PutFragments { fragments, .. } => {
                storage::fragments_to_kv_ops(sed_id, &fragments)
            }
            _ => vec![],
        }
    }

    /// Feed back a completed storage result.
    ///
    /// Returns `Some(StorageComplete)` when a logical operation finishes,
    /// `None` if more results are still pending.
    ///
    /// May also return additional `IssuedOp`s for multi-phase operations
    /// (e.g., Phase 1 load completes → Phase 2 loads needed).
    pub fn handle_result(
        &mut self,
        op_id: OpId,
        result: KvResult,
    ) -> HandleResultOutput {
        // Check if this is a sub-task of a parent operation
        let parent_id = self.subtask_to_parent.remove(&op_id).unwrap_or(op_id);

        let Some(op) = self.pending.get_mut(&parent_id) else {
            return HandleResultOutput::default();
        };

        match op {
            PendingOp::LoadSedimentree { .. } => {
                if let KvResult::LoadRange { values } = result {
                    let op = self.pending.remove(&parent_id).unwrap();
                    if let PendingOp::LoadSedimentree { sed_id, session, .. } = op {
                        let io_result = reassemble_sedimentree_from_range(sed_id, values);
                        return HandleResultOutput {
                            complete: Some(StorageComplete { session, result: io_result }),
                            ..Default::default()
                        };
                    }
                }
            }
            PendingOp::PutData { remaining, .. } => {
                *remaining = remaining.saturating_sub(1);
                if *remaining == 0 {
                    let op = self.pending.remove(&parent_id).unwrap();
                    if let PendingOp::PutData { session, .. } = op {
                        return HandleResultOutput {
                            complete: Some(StorageComplete {
                                session,
                                result: BatchSyncIoResult::PutComplete,
                            }),
                            ..Default::default()
                        };
                    }
                }
            }
            PendingOp::LoadCommitBlobsPhase1 {
                remaining, loaded, task_digests, ..
            } => {
                if let KvResult::Load { value: Some(bytes) } = result {
                    if let Some(digest) = task_digests.remove(&op_id) {
                        loaded.insert(digest, bytes);
                    }
                }
                *remaining = remaining.saturating_sub(1);
                if *remaining == 0 {
                    let op = self.pending.remove(&parent_id).unwrap();
                    if let PendingOp::LoadCommitBlobsPhase1 {
                        sed_id, loaded, session, ..
                    } = op {
                        return self.start_commit_phase2(parent_id, sed_id, loaded, session);
                    }
                }
            }
            PendingOp::LoadCommitBlobsPhase2 {
                remaining, loaded_blobs, task_digests, ..
            } => {
                if let KvResult::Load { value: Some(bytes) } = result {
                    if let Some(commit_digest) = task_digests.remove(&op_id) {
                        loaded_blobs.insert(commit_digest, bytes);
                    }
                }
                *remaining = remaining.saturating_sub(1);
                if *remaining == 0 {
                    let op = self.pending.remove(&parent_id).unwrap();
                    if let PendingOp::LoadCommitBlobsPhase2 {
                        signed_commits, loaded_blobs, session, ..
                    } = op {
                        let blobs = assemble_commit_pairs(signed_commits, loaded_blobs);
                        return HandleResultOutput {
                            complete: Some(StorageComplete {
                                session,
                                result: BatchSyncIoResult::CommitBlobs { blobs },
                            }),
                            ..Default::default()
                        };
                    }
                }
            }
            PendingOp::LoadFragmentBlobsPhase1 {
                remaining, loaded, task_digests, ..
            } => {
                if let KvResult::Load { value: Some(bytes) } = result {
                    if let Some(digest) = task_digests.remove(&op_id) {
                        loaded.insert(digest, bytes);
                    }
                }
                *remaining = remaining.saturating_sub(1);
                if *remaining == 0 {
                    let op = self.pending.remove(&parent_id).unwrap();
                    if let PendingOp::LoadFragmentBlobsPhase1 {
                        sed_id, loaded, session, ..
                    } = op {
                        return self.start_fragment_phase2(parent_id, sed_id, loaded, session);
                    }
                }
            }
            PendingOp::LoadFragmentBlobsPhase2 {
                remaining, loaded_blobs, task_digests, ..
            } => {
                if let KvResult::Load { value: Some(bytes) } = result {
                    if let Some(frag_digest) = task_digests.remove(&op_id) {
                        loaded_blobs.insert(frag_digest, bytes);
                    }
                }
                *remaining = remaining.saturating_sub(1);
                if *remaining == 0 {
                    let op = self.pending.remove(&parent_id).unwrap();
                    if let PendingOp::LoadFragmentBlobsPhase2 {
                        signed_fragments, loaded_blobs, session, ..
                    } = op {
                        let blobs = assemble_fragment_pairs(signed_fragments, loaded_blobs);
                        return HandleResultOutput {
                            complete: Some(StorageComplete {
                                session,
                                result: BatchSyncIoResult::FragmentBlobs { blobs },
                            }),
                            ..Default::default()
                        };
                    }
                }
            }
        }

        HandleResultOutput::default()
    }

    // ----- Internal helpers -----

    fn issue_puts(
        &mut self,
        _sed_id: SedimentreeId,
        session: BatchSyncSession,
        kv_ops: Vec<KvOp>,
    ) -> Vec<IssuedOp> {
        let parent_id = OpId::new();
        let count = kv_ops.len();
        let mut issued = Vec::with_capacity(count);

        for op in kv_ops {
            let sub_id = OpId::new();
            self.subtask_to_parent.insert(sub_id, parent_id);
            issued.push(IssuedOp {
                op_id: sub_id,
                operation: op,
            });
        }

        self.pending.insert(parent_id, PendingOp::PutData {
            remaining: count,
            session,
        });
        issued
    }

    fn start_commit_phase2(
        &mut self,
        _old_parent: OpId,
        sed_id: SedimentreeId,
        loaded_commits: HashMap<Digest<LooseCommit>, Vec<u8>>,
        session: BatchSyncSession,
    ) -> HandleResultOutput {
        let new_parent = OpId::new();
        let mut ops = Vec::new();
        let mut task_digests = HashMap::new();
        let mut signed_commits = HashMap::new();

        for (commit_digest, bytes) in &loaded_commits {
            if let Ok(signed) = Signed::<LooseCommit>::try_decode(bytes.clone()) {
                if let Ok(verified) = signed.try_verify() {
                    let blob_digest = verified.payload().blob_meta().digest();
                    let sub_id = OpId::new();
                    self.subtask_to_parent.insert(sub_id, new_parent);
                    task_digests.insert(sub_id, *commit_digest);
                    ops.push(IssuedOp {
                        op_id: sub_id,
                        operation: KvOp::Load {
                            key: sedimentree_blob_path(&sed_id, &blob_digest),
                        },
                    });
                    signed_commits.insert(*commit_digest, bytes.clone());
                }
            }
        }

        if ops.is_empty() {
            return HandleResultOutput {
                complete: Some(StorageComplete {
                    session,
                    result: BatchSyncIoResult::CommitBlobs { blobs: vec![] },
                }),
                ..Default::default()
            };
        }

        let remaining = ops.len();
        self.pending.insert(new_parent, PendingOp::LoadCommitBlobsPhase2 {
            signed_commits,
            task_digests,
            loaded_blobs: HashMap::new(),
            remaining,
            session,
        });

        HandleResultOutput { new_ops: ops, ..Default::default() }
    }

    fn start_fragment_phase2(
        &mut self,
        _old_parent: OpId,
        sed_id: SedimentreeId,
        loaded_fragments: HashMap<Digest<Fragment>, Vec<u8>>,
        session: BatchSyncSession,
    ) -> HandleResultOutput {
        let new_parent = OpId::new();
        let mut ops = Vec::new();
        let mut task_digests = HashMap::new();
        let mut signed_fragments = HashMap::new();

        for (frag_digest, bytes) in &loaded_fragments {
            if let Ok(signed) = Signed::<Fragment>::try_decode(bytes.clone()) {
                if let Ok(verified) = signed.try_verify() {
                    let blob_digest = verified.payload().summary().blob_meta().digest();
                    let sub_id = OpId::new();
                    self.subtask_to_parent.insert(sub_id, new_parent);
                    task_digests.insert(sub_id, *frag_digest);
                    ops.push(IssuedOp {
                        op_id: sub_id,
                        operation: KvOp::Load {
                            key: sedimentree_blob_path(&sed_id, &blob_digest),
                        },
                    });
                    signed_fragments.insert(*frag_digest, bytes.clone());
                }
            }
        }

        if ops.is_empty() {
            return HandleResultOutput {
                complete: Some(StorageComplete {
                    session,
                    result: BatchSyncIoResult::FragmentBlobs { blobs: vec![] },
                }),
                ..Default::default()
            };
        }

        let remaining = ops.len();
        self.pending.insert(new_parent, PendingOp::LoadFragmentBlobsPhase2 {
            signed_fragments,
            task_digests,
            loaded_blobs: HashMap::new(),
            remaining,
            session,
        });

        HandleResultOutput { new_ops: ops, ..Default::default() }
    }
}

/// Output from `handle_result` — may contain a completed operation
/// and/or new operations to issue (for multi-phase loads).
#[derive(Debug, Default)]
pub struct HandleResultOutput {
    /// A completed logical operation (if any).
    pub complete: Option<StorageComplete>,
    /// Additional operations to issue (e.g., Phase 2 blob loads).
    pub new_ops: Vec<IssuedOp>,
}
