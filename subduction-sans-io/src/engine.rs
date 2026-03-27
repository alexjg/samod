//! The unified subduction protocol state machine.
//!
//! [`SubductionEngine`] owns all protocol state and is driven by
//! [`Input`] events, producing [`EngineOutput`] with IO to execute.
//! The caller is responsible for executing IO and feeding results back.
//!
//! This is a pure sans-IO state machine — no threads, no async, no IO.

use std::collections::{BTreeSet, HashMap};
use std::fmt::Debug;
use std::hash::Hash;

use sedimentree_core::{
    blob::Blob,
    crypto::{
        digest::Digest,
        fingerprint::FingerprintSeed,
    },
    id::SedimentreeId,
    loose_commit::LooseCommit,
    sedimentree::Sedimentree,
};
use subduction_crypto::signed::Signed;

use crate::{
    batch_sync::{BatchSyncIo, BatchSyncSession, BatchSyncStep},
    handshake::{
        HandshakeConfig, HandshakeInput, HandshakeMachine, HandshakeOutput, ResponderConfig,
    },
    incremental::IncrementalSync,
    messages::SyncMessage,
    storage::{self, KvOp, KvResult},
    storage_coord::{IssuedOp, OpId, StorageCoordinator},
    types::{Audience, PeerId as SubPeerId, RemoteHeads, RequestId, TimestampSeconds},
};

// ---------------------------------------------------------------------------
// Public types
// ---------------------------------------------------------------------------

/// Configuration for the engine.
#[derive(Debug, Clone)]
pub struct EngineConfig {
    pub our_verifying_key: ed25519_dalek::VerifyingKey,
    pub responder_config: Option<ResponderConfig>,
}

/// Input events to the engine.
#[derive(Debug)]
pub enum Input<C> {
    /// A new subduction connection was created.
    NewConnection {
        id: C,
        outgoing: bool,
        audience: Audience,
        now: TimestampSeconds,
    },
    /// Bytes received on a connection.
    ReceivedBytes {
        id: C,
        bytes: Vec<u8>,
        now: TimestampSeconds,
    },
    /// A connection was lost.
    ConnectionLost { id: C },
    /// A signing operation completed.
    SigningComplete {
        op_id: OpId,
        signature: ed25519_dalek::Signature,
    },
    /// A storage operation completed.
    StorageComplete {
        op_id: OpId,
        result: KvResult,
    },
    /// Request to find a document via subduction.
    FindDocument {
        sed_id: SedimentreeId,
    },
    /// New local changes to propagate via subduction.
    NewLocalChanges {
        sed_id: SedimentreeId,
        changes: Vec<LocalChange>,
    },
}

/// A local change to propagate via subduction.
#[derive(Debug, Clone)]
pub struct LocalChange {
    /// Parent commit digests.
    pub parents: BTreeSet<Digest<LooseCommit>>,
    /// The raw change bytes (becomes the blob).
    pub blob: Blob,
}

/// A request for the caller to sign payload bytes.
#[derive(Debug, Clone)]
pub struct SignRequest {
    pub op_id: OpId,
    pub payload_bytes: Vec<u8>,
}

/// Status of the engine's search for a document.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SearchStatus {
    Searching,
    Found,
    NotFound,
}

/// Output from processing an input event.
#[derive(Debug)]
pub struct EngineOutput<C> {
    /// Bytes to send on connections.
    pub send: Vec<(C, Vec<u8>)>,
    /// Storage operations to execute.
    pub storage_ops: Vec<IssuedOp>,
    /// Signing requests.
    pub sign_requests: Vec<SignRequest>,
    /// Data to deliver to documents (sed_id → blob bytes).
    pub data_for_docs: Vec<(SedimentreeId, Vec<Vec<u8>>)>,
    /// Search status updates (sed_id → status).
    pub search_status: Vec<(SedimentreeId, SearchStatus)>,
}

impl<C> Default for EngineOutput<C> {
    fn default() -> Self {
        Self {
            send: Vec::new(),
            storage_ops: Vec::new(),
            sign_requests: Vec::new(),
            data_for_docs: Vec::new(),
            search_status: Vec::new(),
        }
    }
}

impl<C> EngineOutput<C> {
    pub fn is_empty(&self) -> bool {
        self.send.is_empty()
            && self.storage_ops.is_empty()
            && self.sign_requests.is_empty()
            && self.data_for_docs.is_empty()
            && self.search_status.is_empty()
    }
}

// ---------------------------------------------------------------------------
// Internal types
// ---------------------------------------------------------------------------

#[derive(Debug)]
enum ConnectionState<C> {
    Handshaking(HandshakeMachine),
    Authenticated { peer_id: SubPeerId, _conn_id: C },
}

#[derive(Debug)]
enum PendingSign<C> {
    Handshake { connection_id: C },
    CommitSign {
        sed_id: SedimentreeId,
        commit: LooseCommit,
        blob: Blob,
    },
}

// ---------------------------------------------------------------------------
// SubductionEngine
// ---------------------------------------------------------------------------

/// The unified subduction protocol state machine.
///
/// Generic over `C` — the caller's connection ID type.
#[derive(Debug)]
pub struct SubductionEngine<C: Eq + Hash + Copy + Debug> {
    config: EngineConfig,
    connections: HashMap<C, ConnectionState<C>>,
    pending_signs: HashMap<OpId, PendingSign<C>>,
    requestor_sessions: HashMap<RequestId, BatchSyncSession>,
    storage_coord: StorageCoordinator,
    incremental: IncrementalSync,
    finding_docs: HashMap<SedimentreeId, ()>,
}

impl<C: Eq + Hash + Copy + Debug> SubductionEngine<C> {
    pub fn new(config: EngineConfig) -> Self {
        Self {
            config,
            connections: HashMap::new(),
            pending_signs: HashMap::new(),
            requestor_sessions: HashMap::new(),
            storage_coord: StorageCoordinator::new(),
            incremental: IncrementalSync::new(),
            finding_docs: HashMap::new(),
        }
    }

    /// Process one input event and return the resulting IO to execute.
    pub fn handle(&mut self, input: Input<C>) -> EngineOutput<C> {
        let mut out = EngineOutput::default();
        match input {
            Input::NewConnection { id, outgoing, audience, now } => {
                self.handle_new_connection(id, outgoing, audience, now, &mut out);
            }
            Input::ReceivedBytes { id, bytes, now } => {
                self.handle_received_bytes(id, bytes, now, &mut out);
            }
            Input::ConnectionLost { id } => {
                self.handle_connection_lost(id);
            }
            Input::SigningComplete { op_id, signature } => {
                self.handle_signing_complete(op_id, signature, &mut out);
            }
            Input::StorageComplete { op_id, result } => {
                self.handle_storage_complete(op_id, result, &mut out);
            }
            Input::FindDocument { sed_id } => {
                self.handle_find_document(sed_id, &mut out);
            }
            Input::NewLocalChanges { sed_id, changes } => {
                self.handle_new_local_changes(sed_id, changes, &mut out);
            }
        }
        out
    }

    // ----- Connection lifecycle -----

    fn handle_new_connection(
        &mut self,
        id: C,
        outgoing: bool,
        audience: Audience,
        now: TimestampSeconds,
        out: &mut EngineOutput<C>,
    ) {
        let handshake_config = HandshakeConfig {
            our_verifying_key: self.config.our_verifying_key,
            responder_config: self.config.responder_config.clone(),
        };
        let mut machine = HandshakeMachine::new(handshake_config);

        if outgoing {
            let nonce = subduction_crypto::nonce::Nonce::from_bytes(rand::random());
            let outputs = machine.step(HandshakeInput::Initiate {
                audience, timestamp: now, nonce,
            });
            self.process_handshake_outputs(id, outputs, out);
        }

        self.connections.insert(id, ConnectionState::Handshaking(machine));
    }

    fn handle_received_bytes(
        &mut self,
        id: C,
        bytes: Vec<u8>,
        now: TimestampSeconds,
        out: &mut EngineOutput<C>,
    ) {
        let Some(conn) = self.connections.get_mut(&id) else { return };
        match conn {
            ConnectionState::Handshaking(machine) => {
                let outputs = machine.step(HandshakeInput::ReceivedBytes { bytes, now });
                self.process_handshake_outputs(id, outputs, out);
            }
            ConnectionState::Authenticated { peer_id, .. } => {
                let peer_id = *peer_id;
                self.handle_sync_bytes(id, peer_id, bytes, out);
            }
        }
    }

    fn handle_connection_lost(&mut self, id: C) {
        if let Some(ConnectionState::Authenticated { peer_id, .. }) = self.connections.remove(&id) {
            self.incremental.peer_disconnected(peer_id);
        } else {
            self.connections.remove(&id);
        }
    }

    // ----- Signing -----

    fn handle_signing_complete(
        &mut self,
        op_id: OpId,
        signature: ed25519_dalek::Signature,
        out: &mut EngineOutput<C>,
    ) {
        let Some(pending) = self.pending_signs.remove(&op_id) else { return };
        match pending {
            PendingSign::Handshake { connection_id } => {
                let Some(ConnectionState::Handshaking(machine)) =
                    self.connections.get_mut(&connection_id)
                else { return };
                let outputs = machine.step(HandshakeInput::SigningComplete { signature });
                self.process_handshake_outputs(connection_id, outputs, out);
            }
            PendingSign::CommitSign { sed_id, commit, blob } => {
                let signed = Signed::from_parts(
                    self.config.our_verifying_key,
                    signature,
                    &commit,
                );
                let commit_digest = Digest::hash(&commit);

                // Store (fire-and-forget)
                for op in storage::commits_to_kv_ops(&sed_id, &[(signed.clone(), blob.clone())]) {
                    out.storage_ops.push(IssuedOp {
                        op_id: OpId::new(),
                        operation: op,
                    });
                }

                // Push to incremental subscribers
                let step = self.incremental.on_local_commit(
                    sed_id, signed, blob, vec![commit_digest],
                );
                self.process_incremental_step(step, out);
            }
        }
    }

    // ----- Storage -----

    fn handle_storage_complete(
        &mut self,
        op_id: OpId,
        result: KvResult,
        out: &mut EngineOutput<C>,
    ) {
        let coord_output = self.storage_coord.handle_result(op_id, result);

        for issued in coord_output.new_ops {
            out.storage_ops.push(issued);
        }

        if let Some(complete) = coord_output.complete {
            let mut session = complete.session;
            let step = session.handle_io_result(complete.result);
            self.process_batch_step(session, step, out);
        }
    }

    // ----- Document finding -----

    fn handle_find_document(
        &mut self,
        sed_id: SedimentreeId,
        out: &mut EngineOutput<C>,
    ) {
        self.finding_docs.insert(sed_id, ());

        let has_authenticated = self.connections.values().any(|s| {
            matches!(s, ConnectionState::Authenticated { .. })
        });

        if has_authenticated {
            self.initiate_batch_sync_for(sed_id, out);
        } else if self.connections.is_empty() {
            // No subduction connections at all — report NotFound immediately.
            // If a connection appears later, retry_pending_finds will re-search.
            out.search_status.push((sed_id, SearchStatus::NotFound));
        }
        // Otherwise there are handshaking connections; retry_pending_finds
        // will pick this up once they authenticate.
    }

    fn retry_pending_finds(&mut self, out: &mut EngineOutput<C>) {
        let needs_retry: Vec<SedimentreeId> = self
            .finding_docs
            .keys()
            .filter(|sed_id| {
                !self.requestor_sessions.values().any(|s| s.sedimentree_id() == **sed_id)
            })
            .copied()
            .collect();

        for sed_id in needs_retry {
            self.initiate_batch_sync_for(sed_id, out);
        }
    }

    fn initiate_batch_sync_for(&mut self, sed_id: SedimentreeId, out: &mut EngineOutput<C>) {
        let conn_id = self.connections.iter().find_map(|(id, state)| {
            matches!(state, ConnectionState::Authenticated { .. }).then_some(*id)
        });
        let Some(conn_id) = conn_id else { return };

        let local_tree = Sedimentree::new(vec![], vec![]);
        let seed = FingerprintSeed::new(rand::random(), rand::random());
        let req_id = RequestId {
            requestor: SubPeerId::new(self.config.our_verifying_key.to_bytes()),
            nonce: rand::random(),
        };
        let our_heads = RemoteHeads { counter: 0, heads: vec![] };

        let (session, msg) = BatchSyncSession::initiate(
            sed_id, &local_tree, req_id, true, seed, our_heads,
        );

        out.send.push((conn_id, msg.encode()));
        self.requestor_sessions.insert(req_id, session);
    }

    // ----- Local changes -----

    fn handle_new_local_changes(
        &mut self,
        sed_id: SedimentreeId,
        changes: Vec<LocalChange>,
        out: &mut EngineOutput<C>,
    ) {
        for change in &changes {
            let blob_meta = change.blob.meta();
            let commit = LooseCommit::new(sed_id, change.parents.clone(), blob_meta);

            // Store blob (fire-and-forget)
            let blob_digest = change.blob.meta().digest();
            out.storage_ops.push(IssuedOp {
                op_id: OpId::new(),
                operation: KvOp::Put {
                    key: storage::sedimentree_blob_path(&sed_id, &blob_digest),
                    value: change.blob.as_slice().to_vec(),
                },
            });

            // Request signing
            use sedimentree_core::codec::{encode::EncodeFields, schema::Schema};
            let mut payload_bytes = Vec::new();
            payload_bytes.extend_from_slice(&LooseCommit::SCHEMA);
            payload_bytes.extend_from_slice(self.config.our_verifying_key.as_bytes());
            commit.encode_fields(&mut payload_bytes);

            let sign_op_id = OpId::new();
            out.sign_requests.push(SignRequest {
                op_id: sign_op_id,
                payload_bytes: payload_bytes.clone(),
            });
            self.pending_signs.insert(sign_op_id, PendingSign::CommitSign {
                sed_id, commit, blob: change.blob.clone(),
            });
        }
    }

    // ----- Handshake processing -----

    fn process_handshake_outputs(
        &mut self,
        conn_id: C,
        outputs: Vec<HandshakeOutput>,
        out: &mut EngineOutput<C>,
    ) {
        for output in outputs {
            match output {
                HandshakeOutput::SendBytes(bytes) => {
                    out.send.push((conn_id, bytes));
                }
                HandshakeOutput::SigningRequest { payload_bytes } => {
                    let op_id = OpId::new();
                    out.sign_requests.push(SignRequest {
                        op_id,
                        payload_bytes,
                    });
                    self.pending_signs.insert(op_id, PendingSign::Handshake {
                        connection_id: conn_id,
                    });
                }
                HandshakeOutput::Complete { their_peer_id } => {
                    tracing::info!(?conn_id, ?their_peer_id, "subduction handshake complete");
                    self.connections.insert(
                        conn_id,
                        ConnectionState::Authenticated {
                            peer_id: their_peer_id,
                            _conn_id: conn_id,
                        },
                    );
                    self.retry_pending_finds(out);
                }
                HandshakeOutput::Failed(err) => {
                    tracing::warn!(?conn_id, ?err, "subduction handshake failed");
                    self.connections.remove(&conn_id);
                }
            }
        }
    }

    // ----- Sync message handling -----

    fn handle_sync_bytes(
        &mut self,
        conn_id: C,
        peer_id: SubPeerId,
        bytes: Vec<u8>,
        out: &mut EngineOutput<C>,
    ) {
        let msg = match SyncMessage::try_decode(&bytes) {
            Ok(msg) => msg,
            Err(e) => {
                tracing::warn!(?conn_id, ?e, "failed to decode subduction sync message");
                return;
            }
        };

        match msg {
            SyncMessage::BatchSyncRequest(request) => {
                if request.subscribe {
                    self.incremental.subscribe(peer_id, request.id);
                }
                let (session, step) = BatchSyncSession::respond(request);
                self.process_batch_step(session, step, out);
            }
            SyncMessage::BatchSyncResponse(response) => {
                if let Some(mut session) = self.requestor_sessions.remove(&response.req_id) {
                    let step = session.handle_message(SyncMessage::BatchSyncResponse(response));
                    self.process_batch_step(session, step, out);
                }
            }
            SyncMessage::LooseCommit { id, commit, blob, sender_heads } => {
                if commit.try_verify().is_err() { return; }
                let commit_digest = storage::compute_commit_digest(&commit);
                for op in storage::commits_to_kv_ops(&id, &[(commit.clone(), blob.clone())]) {
                    out.storage_ops.push(IssuedOp { op_id: OpId::new(), operation: op });
                }
                let step = self.incremental.on_received_commit(
                    peer_id, id, commit, blob, sender_heads, vec![commit_digest],
                );
                self.process_incremental_step(step, out);
            }
            SyncMessage::Fragment { id, fragment, blob, sender_heads } => {
                if fragment.try_verify().is_err() { return; }
                for op in storage::fragments_to_kv_ops(&id, &[(fragment.clone(), blob.clone())]) {
                    out.storage_ops.push(IssuedOp { op_id: OpId::new(), operation: op });
                }
                let step = self.incremental.on_received_fragment(
                    peer_id, id, fragment, blob, sender_heads, vec![],
                );
                self.process_incremental_step(step, out);
            }
            SyncMessage::HeadsUpdate { id, heads } => {
                let step = self.incremental.on_heads_update(peer_id, id, heads);
                self.process_incremental_step(step, out);
            }
            SyncMessage::RemoveSubscriptions(unsub) => {
                self.incremental.unsubscribe(peer_id, &unsub.ids);
            }
            _ => {}
        }
    }

    // ----- Batch sync step processing -----

    fn process_batch_step(
        &mut self,
        session: BatchSyncSession,
        step: BatchSyncStep,
        out: &mut EngineOutput<C>,
    ) {
        let sed_id = session.sedimentree_id();

        // Send messages
        for msg in step.messages {
            let encoded = msg.encode();
            if let Some((&conn_id, _)) = self.connections.iter().find(|(_, s)| {
                matches!(s, ConnectionState::Authenticated { .. })
            }) {
                out.send.push((conn_id, encoded));
            }
        }

        // Extract blob bytes for document delivery
        let mut blobs_for_doc = Vec::new();
        for io_task in &step.io_tasks {
            match io_task {
                BatchSyncIo::PutCommits { commits, .. } => {
                    for (_, blob) in commits {
                        blobs_for_doc.push(blob.as_slice().to_vec());
                    }
                }
                BatchSyncIo::PutFragments { fragments, .. } => {
                    for (_, blob) in fragments {
                        blobs_for_doc.push(blob.as_slice().to_vec());
                    }
                }
                _ => {}
            }
        }

        if step.complete && self.finding_docs.contains_key(&sed_id) {
            if blobs_for_doc.is_empty() {
                out.search_status.push((sed_id, SearchStatus::NotFound));
            } else {
                out.data_for_docs.push((sed_id, blobs_for_doc));
                out.search_status.push((sed_id, SearchStatus::Found));
            }
        } else if !blobs_for_doc.is_empty() && self.finding_docs.contains_key(&sed_id) {
            out.data_for_docs.push((sed_id, blobs_for_doc));
        }

        // Issue storage via coordinator
        for io_task in step.io_tasks {
            if session.is_complete() {
                for op in self.storage_coord.issue_fire_and_forget(&sed_id, io_task) {
                    out.storage_ops.push(IssuedOp { op_id: OpId::new(), operation: op });
                }
            } else {
                let issued = self.storage_coord.issue(session.clone(), io_task);
                out.storage_ops.extend(issued);
            }
        }
    }

    fn process_incremental_step(
        &mut self,
        step: crate::incremental::IncrementalStep,
        out: &mut EngineOutput<C>,
    ) {
        for (peer_id, msg) in step.messages {
            let encoded = msg.encode();
            if let Some((&conn_id, _)) = self.connections.iter()
                .find(|(_, s)| matches!(s, ConnectionState::Authenticated { peer_id: p, .. } if *p == peer_id))
            {
                out.send.push((conn_id, encoded));
            }
        }
    }
}
