//! Batch sync session: fingerprint-based reconciliation in 1.5 round trips.
//!
//! A [`BatchSyncSession`] manages one batch sync exchange for a single
//! sedimentree between two peers. It handles both the requestor and
//! responder roles as a sans-IO state machine.
//!
//! # Requestor flow
//!
//! 1. Call [`BatchSyncSession::initiate`] with the local sedimentree
//! 2. Send the returned `BatchSyncRequest` message
//! 3. Feed the `BatchSyncResponse` via [`handle_message`](BatchSyncSession::handle_message)
//! 4. Execute returned IO tasks (store received data, load requested blobs)
//! 5. Feed IO results via [`handle_io_result`](BatchSyncSession::handle_io_result)
//! 6. Send returned fire-and-forget messages
//!
//! # Responder flow
//!
//! 1. Call [`BatchSyncSession::respond`] with the received request
//! 2. Execute the returned `LoadSedimentree` IO task
//! 3. Feed the result via [`handle_io_result`](BatchSyncSession::handle_io_result)
//! 4. Send the returned `BatchSyncResponse` message

use std::collections::HashMap;

use sedimentree_core::{
    blob::Blob,
    commit::CountLeadingZeroBytes,
    crypto::{
        digest::Digest,
        fingerprint::{Fingerprint, FingerprintSeed},
    },
    fragment::{Fragment, id::FragmentId},
    id::SedimentreeId,
    loose_commit::{LooseCommit, id::CommitId},
    sedimentree::Sedimentree,
};
use subduction_crypto::signed::Signed;

use crate::{
    messages::{
        BatchSyncRequest, BatchSyncResponse, RequestedData, SyncDiff,
        SyncMessage, SyncResult,
    },
    types::{RemoteHeads, RequestId},
};

// ---------------------------------------------------------------------------
// IO types
// ---------------------------------------------------------------------------

/// IO tasks the batch sync session may request.
#[derive(Debug, Clone)]
pub enum BatchSyncIo {
    /// Load the full sedimentree data for a document.
    ///
    /// The result must include the `Sedimentree` metadata and all
    /// `(Signed<T>, Blob)` pairs keyed by digest.
    LoadSedimentree { id: SedimentreeId },

    /// Load specific commit blobs by digest.
    LoadCommitBlobs {
        id: SedimentreeId,
        digests: Vec<Digest<LooseCommit>>,
    },

    /// Load specific fragment blobs by digest.
    LoadFragmentBlobs {
        id: SedimentreeId,
        digests: Vec<Digest<Fragment>>,
    },

    /// Store received commits with their blobs.
    PutCommits {
        id: SedimentreeId,
        commits: Vec<(Signed<LooseCommit>, Blob)>,
    },

    /// Store received fragments with their blobs.
    PutFragments {
        id: SedimentreeId,
        fragments: Vec<(Signed<Fragment>, Blob)>,
    },
}

/// Results of IO tasks, fed back into the session.
#[derive(Debug, Clone)]
pub enum BatchSyncIoResult {
    /// Result of `LoadSedimentree`.
    Sedimentree {
        id: SedimentreeId,
        sedimentree: Sedimentree,
        commits: HashMap<Digest<LooseCommit>, (Signed<LooseCommit>, Blob)>,
        fragments: HashMap<Digest<Fragment>, (Signed<Fragment>, Blob)>,
    },

    /// Result of `LoadCommitBlobs`.
    CommitBlobs {
        blobs: Vec<(Signed<LooseCommit>, Blob)>,
    },

    /// Result of `LoadFragmentBlobs`.
    FragmentBlobs {
        blobs: Vec<(Signed<Fragment>, Blob)>,
    },

    /// Confirmation that `PutCommits` or `PutFragments` completed.
    PutComplete,
}

/// The result of a session step — what the caller should do next.
#[derive(Debug, Clone, Default)]
pub struct BatchSyncStep {
    /// Messages to send to the peer.
    pub messages: Vec<SyncMessage>,
    /// IO tasks to execute.
    pub io_tasks: Vec<BatchSyncIo>,
    /// Remote heads received from the peer (if any).
    pub remote_heads: Option<RemoteHeads>,
    /// Whether the session completed (successfully or with an error).
    pub complete: bool,
    /// If the session ended due to NotFound or Unauthorized.
    pub error: Option<BatchSyncError>,
}

/// Errors that can terminate a batch sync session.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BatchSyncError {
    /// The responder doesn't have this sedimentree.
    NotFound,
    /// We're not authorized to access this sedimentree.
    Unauthorized,
    /// Received an unexpected message for the current state.
    UnexpectedMessage,
    /// Received an IO result in an unexpected state.
    UnexpectedIoResult,
}

// ---------------------------------------------------------------------------
// Session
// ---------------------------------------------------------------------------

/// A batch sync session for one sedimentree between two peers.
#[derive(Debug, Clone)]
pub struct BatchSyncSession {
    id: SedimentreeId,
    state: State,
}

#[derive(Debug, Clone)]
enum State {
    /// Requestor: we sent a request, waiting for the response.
    RequestorAwaitingResponse {
        seed: FingerprintSeed,
        /// The local sedimentree, kept for reverse-lookup during fire-and-forget.
        local_tree: Sedimentree,
        /// Our current heads to attach to fire-and-forget messages.
        our_heads: RemoteHeads,
    },

    /// Requestor: we got the response, stored received data, now loading
    /// local blobs for items the responder requested.
    RequestorLoadingBlobs {
        /// Number of pending blob load IO tasks (commits + fragments).
        pending_loads: u32,
        /// Messages accumulated so far (from completed blob loads).
        pending_messages: Vec<SyncMessage>,
        our_heads: RemoteHeads,
    },

    /// Responder: we received a request, loading local sedimentree.
    ResponderLoading {
        request: BatchSyncRequest,
    },

    /// Session is done.
    Complete,

    /// Session failed.
    Failed,
}

impl BatchSyncSession {
    /// Create a requestor session and build the `BatchSyncRequest` to send.
    ///
    /// The caller provides the local sedimentree (already loaded in memory),
    /// a random fingerprint seed, and the current heads for this sedimentree.
    pub fn initiate(
        id: SedimentreeId,
        local_tree: &Sedimentree,
        req_id: RequestId,
        subscribe: bool,
        seed: FingerprintSeed,
        our_heads: RemoteHeads,
    ) -> (Self, SyncMessage) {
        let fingerprint_summary = local_tree.fingerprint_summarize(&seed);

        let msg = SyncMessage::BatchSyncRequest(BatchSyncRequest {
            id,
            req_id,
            fingerprint_summary,
            subscribe,
        });

        let session = Self {
            id,
            state: State::RequestorAwaitingResponse {
                seed,
                local_tree: local_tree.clone(),
                our_heads,
            },
        };

        (session, msg)
    }

    /// Create a responder session from a received `BatchSyncRequest`.
    ///
    /// Returns the session and an IO task to load the local sedimentree.
    pub fn respond(request: BatchSyncRequest) -> (Self, BatchSyncStep) {
        let id = request.id;
        let session = Self {
            id,
            state: State::ResponderLoading { request },
        };

        let step = BatchSyncStep {
            io_tasks: vec![BatchSyncIo::LoadSedimentree { id }],
            ..Default::default()
        };

        (session, step)
    }

    /// The sedimentree ID this session is syncing.
    pub fn sedimentree_id(&self) -> SedimentreeId {
        self.id
    }

    /// Whether this session has completed.
    pub fn is_complete(&self) -> bool {
        matches!(self.state, State::Complete | State::Failed)
    }

    /// Feed a received message into the session.
    pub fn handle_message(&mut self, msg: SyncMessage) -> BatchSyncStep {
        match std::mem::replace(&mut self.state, State::Complete) {
            State::RequestorAwaitingResponse {
                seed,
                local_tree,
                our_heads,
            } => match msg {
                SyncMessage::BatchSyncResponse(resp) => {
                    self.handle_response(resp, seed, local_tree, our_heads)
                }
                _ => self.fail(BatchSyncError::UnexpectedMessage),
            },
            other => {
                self.state = other;
                self.fail(BatchSyncError::UnexpectedMessage)
            }
        }
    }

    /// Feed an IO result back into the session.
    pub fn handle_io_result(&mut self, result: BatchSyncIoResult) -> BatchSyncStep {
        match std::mem::replace(&mut self.state, State::Complete) {
            State::ResponderLoading { request } => match result {
                BatchSyncIoResult::Sedimentree {
                    sedimentree,
                    commits,
                    fragments,
                    ..
                } => self.handle_responder_loaded(request, sedimentree, commits, fragments),
                _ => self.fail(BatchSyncError::UnexpectedIoResult),
            },

            State::RequestorLoadingBlobs {
                pending_loads,
                mut pending_messages,
                our_heads,
            } => match result {
                BatchSyncIoResult::CommitBlobs { blobs } => {
                    for (signed, blob) in blobs {
                        pending_messages.push(SyncMessage::LooseCommit {
                            id: self.id,
                            commit: signed,
                            blob,
                            sender_heads: our_heads.clone(),
                        });
                    }
                    self.check_requestor_loads_complete(pending_loads - 1, pending_messages, our_heads)
                }
                BatchSyncIoResult::FragmentBlobs { blobs } => {
                    for (signed, blob) in blobs {
                        pending_messages.push(SyncMessage::Fragment {
                            id: self.id,
                            fragment: signed,
                            blob,
                            sender_heads: our_heads.clone(),
                        });
                    }
                    self.check_requestor_loads_complete(pending_loads - 1, pending_messages, our_heads)
                }
                BatchSyncIoResult::PutComplete => {
                    // Storage of received data confirmed. Continue waiting for blob loads.
                    self.state = State::RequestorLoadingBlobs {
                        pending_loads,
                        pending_messages,
                        our_heads,
                    };
                    BatchSyncStep::default()
                }
                _ => self.fail(BatchSyncError::UnexpectedIoResult),
            },

            other => {
                self.state = other;
                self.fail(BatchSyncError::UnexpectedIoResult)
            }
        }
    }

    // ----- Requestor internals -----

    fn handle_response(
        &mut self,
        resp: BatchSyncResponse,
        seed: FingerprintSeed,
        local_tree: Sedimentree,
        our_heads: RemoteHeads,
    ) -> BatchSyncStep {
        let remote_heads = Some(resp.responder_heads);

        match resp.result {
            SyncResult::NotFound => {
                self.state = State::Failed;
                BatchSyncStep {
                    remote_heads,
                    complete: true,
                    error: Some(BatchSyncError::NotFound),
                    ..Default::default()
                }
            }
            SyncResult::Unauthorized => {
                self.state = State::Failed;
                BatchSyncStep {
                    remote_heads,
                    complete: true,
                    error: Some(BatchSyncError::Unauthorized),
                    ..Default::default()
                }
            }
            SyncResult::Ok(diff) => {
                let mut io_tasks = Vec::new();

                // Store received data
                if !diff.missing_commits.is_empty() {
                    io_tasks.push(BatchSyncIo::PutCommits {
                        id: self.id,
                        commits: diff.missing_commits,
                    });
                }
                if !diff.missing_fragments.is_empty() {
                    io_tasks.push(BatchSyncIo::PutFragments {
                        id: self.id,
                        fragments: diff.missing_fragments,
                    });
                }

                // Reverse-lookup requested fingerprints → local digests
                let (commit_digests, fragment_digests) =
                    reverse_lookup_requested(&local_tree, &seed, &diff.requesting);

                let mut pending_loads = 0u32;
                if !commit_digests.is_empty() {
                    io_tasks.push(BatchSyncIo::LoadCommitBlobs {
                        id: self.id,
                        digests: commit_digests,
                    });
                    pending_loads += 1;
                }
                if !fragment_digests.is_empty() {
                    io_tasks.push(BatchSyncIo::LoadFragmentBlobs {
                        id: self.id,
                        digests: fragment_digests,
                    });
                    pending_loads += 1;
                }

                if pending_loads == 0 {
                    // Nothing to send back — we're done.
                    self.state = State::Complete;
                    BatchSyncStep {
                        remote_heads,
                        io_tasks,
                        complete: true,
                        ..Default::default()
                    }
                } else {
                    self.state = State::RequestorLoadingBlobs {
                        pending_loads,
                        pending_messages: Vec::new(),
                        our_heads,
                    };
                    BatchSyncStep {
                        remote_heads,
                        io_tasks,
                        ..Default::default()
                    }
                }
            }
        }
    }

    fn check_requestor_loads_complete(
        &mut self,
        remaining: u32,
        pending_messages: Vec<SyncMessage>,
        our_heads: RemoteHeads,
    ) -> BatchSyncStep {
        if remaining == 0 {
            self.state = State::Complete;
            BatchSyncStep {
                messages: pending_messages,
                complete: true,
                ..Default::default()
            }
        } else {
            self.state = State::RequestorLoadingBlobs {
                pending_loads: remaining,
                pending_messages,
                our_heads,
            };
            BatchSyncStep::default()
        }
    }

    // ----- Responder internals -----

    fn handle_responder_loaded(
        &mut self,
        request: BatchSyncRequest,
        sedimentree: Sedimentree,
        commits: HashMap<Digest<LooseCommit>, (Signed<LooseCommit>, Blob)>,
        fragments: HashMap<Digest<Fragment>, (Signed<Fragment>, Blob)>,
    ) -> BatchSyncStep {
        let diff = sedimentree.diff_remote_fingerprints(&request.fingerprint_summary);

        // Collect full data for items the requestor is missing
        let missing_commits: Vec<(Signed<LooseCommit>, Blob)> = diff
            .local_only_commits
            .iter()
            .filter_map(|(digest, _)| commits.get(digest).cloned())
            .collect();

        let missing_fragments: Vec<(Signed<Fragment>, Blob)> = diff
            .local_only_fragments
            .iter()
            .filter_map(|(digest, _)| fragments.get(digest).cloned())
            .collect();

        // Echo back fingerprints the requestor has that we don't
        let requesting = RequestedData {
            commit_fingerprints: diff.remote_only_commit_fingerprints,
            fragment_fingerprints: diff.remote_only_fragment_fingerprints,
        };

        // Build current heads
        let depth_metric = CountLeadingZeroBytes;
        let heads = sedimentree.heads(&depth_metric);
        // TODO: the counter should come from the caller (Hub manages per-peer counters)
        let responder_heads = RemoteHeads { counter: 0, heads };

        let response = SyncMessage::BatchSyncResponse(BatchSyncResponse {
            req_id: request.req_id,
            id: self.id,
            result: SyncResult::Ok(SyncDiff {
                missing_commits,
                missing_fragments,
                requesting,
            }),
            responder_heads: responder_heads.clone(),
        });

        self.state = State::Complete;

        BatchSyncStep {
            messages: vec![response],
            remote_heads: Some(responder_heads),
            complete: true,
            ..Default::default()
        }
    }

    fn fail(&mut self, error: BatchSyncError) -> BatchSyncStep {
        self.state = State::Failed;
        BatchSyncStep {
            complete: true,
            error: Some(error),
            ..Default::default()
        }
    }
}

/// Reverse-lookup requested fingerprints to local digests.
///
/// The responder echoed back fingerprints of items it needs. We recompute
/// fingerprints of our local items using the same seed to find matches.
fn reverse_lookup_requested(
    local_tree: &Sedimentree,
    seed: &FingerprintSeed,
    requesting: &RequestedData,
) -> (Vec<Digest<LooseCommit>>, Vec<Digest<Fragment>>) {
    if requesting.commit_fingerprints.is_empty() && requesting.fragment_fingerprints.is_empty() {
        return (Vec::new(), Vec::new());
    }

    // Build fingerprint → digest maps
    let commit_fp_to_digest: HashMap<Fingerprint<CommitId>, Digest<LooseCommit>> = local_tree
        .commit_entries()
        .map(|(digest, c)| (Fingerprint::new(seed, &c.commit_id()), *digest))
        .collect();

    let fragment_fp_to_digest: HashMap<Fingerprint<FragmentId>, Digest<Fragment>> = local_tree
        .fragment_entries()
        .map(|(digest, f)| (Fingerprint::new(seed, &f.fragment_id()), *digest))
        .collect();

    let commit_digests: Vec<_> = requesting
        .commit_fingerprints
        .iter()
        .filter_map(|fp| commit_fp_to_digest.get(fp).copied())
        .collect();

    let fragment_digests: Vec<_> = requesting
        .fragment_fingerprints
        .iter()
        .filter_map(|fp| fragment_fp_to_digest.get(fp).copied())
        .collect();

    (commit_digests, fragment_digests)
}

#[cfg(test)]
mod tests {
    use super::*;
    use ed25519_dalek::{SigningKey, Signer as _};
    use sedimentree_core::{
        blob::BlobMeta,
        codec::{encode::EncodeFields, schema::Schema},
    };

    /// Helper to create a signed loose commit with a blob.
    fn make_signed_commit(
        signing_key: &SigningKey,
        sedimentree_id: SedimentreeId,
        parents: std::collections::BTreeSet<Digest<LooseCommit>>,
        blob_data: Vec<u8>,
    ) -> (Signed<LooseCommit>, Blob, Digest<LooseCommit>) {
        let blob = Blob::new(blob_data);
        let blob_meta = BlobMeta::new(&blob);
        let commit = LooseCommit::new(sedimentree_id, parents, blob_meta);

        let vk = signing_key.verifying_key();
        let mut payload = Vec::new();
        payload.extend_from_slice(&LooseCommit::SCHEMA);
        payload.extend_from_slice(vk.as_bytes());
        commit.encode_fields(&mut payload);
        let sig = signing_key.sign(&payload);
        let signed = Signed::from_parts(vk, sig, &commit);

        let digest = Digest::hash(&commit);
        (signed, blob, digest)
    }

    fn make_keypair(seed: u8) -> SigningKey {
        SigningKey::from_bytes(&[seed; 32])
    }

    fn make_sed_id(val: u8) -> SedimentreeId {
        SedimentreeId::new([val; 32])
    }

    fn make_request_id(val: u8) -> RequestId {
        RequestId {
            requestor: crate::types::PeerId::new([val; 32]),
            nonce: val as u64,
        }
    }

    #[test]
    fn requestor_initiation_produces_request() {
        let sed_id = make_sed_id(1);
        let tree = Sedimentree::new(vec![], vec![]);
        let seed = FingerprintSeed::new(42, 43);
        let req_id = make_request_id(1);
        let heads = RemoteHeads {
            counter: 0,
            heads: vec![],
        };

        let (session, msg) = BatchSyncSession::initiate(sed_id, &tree, req_id, false, seed, heads);

        assert!(!session.is_complete());
        match msg {
            SyncMessage::BatchSyncRequest(req) => {
                assert_eq!(req.id, sed_id);
                assert_eq!(req.req_id, req_id);
                assert!(!req.subscribe);
            }
            other => panic!("expected BatchSyncRequest, got {:?}", other),
        }
    }

    #[test]
    fn responder_not_found() {
        let sed_id = make_sed_id(1);
        let req_id = make_request_id(1);
        let seed = FingerprintSeed::new(1, 2);
        let tree = Sedimentree::new(vec![], vec![]);
        let heads = RemoteHeads {
            counter: 0,
            heads: vec![],
        };

        let (mut requestor, _req_msg) =
            BatchSyncSession::initiate(sed_id, &tree, req_id, false, seed, heads);

        // Simulate responder returning NotFound
        let resp = SyncMessage::BatchSyncResponse(BatchSyncResponse {
            req_id,
            id: sed_id,
            result: SyncResult::NotFound,
            responder_heads: RemoteHeads {
                counter: 0,
                heads: vec![],
            },
        });

        let step = requestor.handle_message(resp);
        assert!(step.complete);
        assert_eq!(step.error, Some(BatchSyncError::NotFound));
        assert!(requestor.is_complete());
    }

    #[test]
    fn full_two_peer_exchange() {
        let signing_key = make_keypair(1);
        let sed_id = make_sed_id(1);

        // Requestor has commit A
        let (signed_a, blob_a, digest_a) = make_signed_commit(
            &signing_key,
            sed_id,
            std::collections::BTreeSet::new(),
            vec![1, 2, 3],
        );

        // Responder has commit B
        let (signed_b, blob_b, digest_b) = make_signed_commit(
            &signing_key,
            sed_id,
            std::collections::BTreeSet::new(),
            vec![4, 5, 6],
        );

        // Build sedimentrees
        let commit_a = signed_a.try_verify().unwrap();
        let commit_b = signed_b.try_verify().unwrap();

        let tree_a = Sedimentree::new(vec![], vec![commit_a.payload().clone()]);
        let tree_b = Sedimentree::new(vec![], vec![commit_b.payload().clone()]);

        // --- Requestor initiates ---
        let seed = FingerprintSeed::new(100, 200);
        let req_id = make_request_id(1);
        let (mut requestor, req_msg) = BatchSyncSession::initiate(
            sed_id,
            &tree_a,
            req_id,
            false,
            seed,
            RemoteHeads {
                counter: 1,
                heads: vec![digest_a],
            },
        );

        // --- Responder receives request ---
        let request = match req_msg {
            SyncMessage::BatchSyncRequest(r) => r,
            _ => panic!("expected BatchSyncRequest"),
        };

        let (mut responder, resp_step) = BatchSyncSession::respond(request.clone());
        assert_eq!(resp_step.io_tasks.len(), 1); // LoadSedimentree

        // --- Responder loads its sedimentree ---
        let mut resp_commits = HashMap::new();
        resp_commits.insert(digest_b, (signed_b.clone(), blob_b.clone()));

        let resp_step = responder.handle_io_result(BatchSyncIoResult::Sedimentree {
            id: sed_id,
            sedimentree: tree_b,
            commits: resp_commits,
            fragments: HashMap::new(),
        });

        assert!(resp_step.complete);
        assert_eq!(resp_step.messages.len(), 1);

        let resp_msg = &resp_step.messages[0];
        let batch_resp = match resp_msg {
            SyncMessage::BatchSyncResponse(r) => r,
            other => panic!("expected BatchSyncResponse, got {:?}", other),
        };

        // Responder should send commit B (which requestor doesn't have)
        match &batch_resp.result {
            SyncResult::Ok(diff) => {
                assert_eq!(diff.missing_commits.len(), 1);
                // Responder should request commit A's fingerprint back
                assert!(!diff.requesting.commit_fingerprints.is_empty());
            }
            other => panic!("expected Ok, got {:?}", other),
        }

        // --- Requestor receives response ---
        let req_step = requestor.handle_message(resp_msg.clone());
        assert!(req_step.remote_heads.is_some());

        // Should have IO tasks: store received commit B + load commit A blobs
        let has_put = req_step
            .io_tasks
            .iter()
            .any(|t| matches!(t, BatchSyncIo::PutCommits { .. }));
        let has_load = req_step
            .io_tasks
            .iter()
            .any(|t| matches!(t, BatchSyncIo::LoadCommitBlobs { .. }));
        assert!(has_put, "should store received commits");
        assert!(has_load, "should load local blobs to send back");

        // --- Requestor: storage confirms ---
        let req_step = requestor.handle_io_result(BatchSyncIoResult::PutComplete);
        assert!(!req_step.complete); // Still waiting for blob load

        // --- Requestor: blob load completes ---
        let req_step = requestor.handle_io_result(BatchSyncIoResult::CommitBlobs {
            blobs: vec![(signed_a.clone(), blob_a.clone())],
        });

        assert!(req_step.complete);
        // Should emit fire-and-forget LooseCommit message with commit A
        assert_eq!(req_step.messages.len(), 1);
        match &req_step.messages[0] {
            SyncMessage::LooseCommit { commit, .. } => {
                assert_eq!(commit.as_bytes(), signed_a.as_bytes());
            }
            other => panic!("expected LooseCommit, got {:?}", other),
        }
    }

    #[test]
    fn requestor_completes_immediately_when_nothing_requested() {
        let signing_key = make_keypair(1);
        let sed_id = make_sed_id(1);

        // Both peers have the same commit
        let (signed_a, _blob_a, digest_a) = make_signed_commit(
            &signing_key,
            sed_id,
            std::collections::BTreeSet::new(),
            vec![1, 2, 3],
        );

        let commit_a = signed_a.try_verify().unwrap();
        let tree = Sedimentree::new(vec![], vec![commit_a.payload().clone()]);

        let seed = FingerprintSeed::new(100, 200);
        let req_id = make_request_id(1);
        let (mut requestor, _) = BatchSyncSession::initiate(
            sed_id,
            &tree,
            req_id,
            false,
            seed,
            RemoteHeads {
                counter: 1,
                heads: vec![],
            },
        );

        // Responder has the same data — nothing missing, nothing requested
        let resp = SyncMessage::BatchSyncResponse(BatchSyncResponse {
            req_id,
            id: sed_id,
            result: SyncResult::Ok(SyncDiff {
                missing_commits: vec![],
                missing_fragments: vec![],
                requesting: RequestedData {
                    commit_fingerprints: vec![],
                    fragment_fingerprints: vec![],
                },
            }),
            responder_heads: RemoteHeads {
                counter: 1,
                heads: vec![digest_a],
            },
        });

        let step = requestor.handle_message(resp);
        assert!(step.complete);
        assert!(step.error.is_none());
        assert!(step.messages.is_empty());
    }
}
