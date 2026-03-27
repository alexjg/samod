//! Thin adapter between samod-core's Hub IO and the subduction protocol engine.
//!
//! All protocol logic lives in [`subduction_sans_io::engine::SubductionEngine`].
//! This module converts between samod-core types (IoTaskId, HubResults, etc.)
//! and the engine's abstract IO types.

use std::collections::HashMap;

use sedimentree_core::{blob::Blob, crypto::digest::Digest, loose_commit::LooseCommit};
use subduction_sans_io::{
    engine::{EngineConfig, EngineOutput, Input, LocalChange, SearchStatus, SubductionEngine},
    handshake::ResponderConfig,
    storage::{KvOp, KvResult, StorageKey as SubStorageKey},
    storage_coord::OpId,
    types::{Audience, TimestampSeconds},
};

use crate::{
    ConnectionId, DocumentId, UnixTimestamp,
    io::{IoTaskId, StorageResult, StorageTask},
};

use super::HubResults;

// ---------------------------------------------------------------------------
// Public types
// ---------------------------------------------------------------------------

/// Configuration for subduction in the Hub.
#[derive(Debug, Clone)]
pub struct SubductionConfig {
    pub our_verifying_key: ed25519_dalek::VerifyingKey,
    pub responder_config: Option<ResponderConfig>,
}

impl SubductionConfig {
    pub fn new<R: rand::Rng>(rng: &mut R) -> Self {
        // Generate a new random verifying key for this Hub instance.
        let signer_bytes: [u8; 32] = rng.random();
        let signing = ed25519_dalek::SigningKey::from_bytes(&signer_bytes);
        Self {
            our_verifying_key: signing.verifying_key(),
            responder_config: None,
        }
    }
}

/// Data that should be sent to a DocumentActor after subduction sync.
#[derive(Debug)]
pub(crate) struct SubductionDataForDoc {
    pub(crate) document_id: DocumentId,
    pub(crate) blobs: Vec<Vec<u8>>,
    pub(crate) status: SearchStatus,
}

// ---------------------------------------------------------------------------
// SubductionSync — thin adapter
// ---------------------------------------------------------------------------

#[derive(Debug)]
pub(crate) struct SubductionSync {
    engine: SubductionEngine<ConnectionId>,
    /// Maps samod IoTaskId → engine OpId for storage and signing results.
    task_to_op: HashMap<IoTaskId, OpId>,
    /// Queued data for DocumentActors.
    pub(crate) data_for_docs: Vec<SubductionDataForDoc>,
}

impl SubductionSync {
    pub(crate) fn new(config: SubductionConfig) -> Self {
        Self {
            engine: SubductionEngine::new(EngineConfig {
                our_verifying_key: config.our_verifying_key,
                responder_config: config.responder_config,
            }),
            task_to_op: HashMap::new(),
            data_for_docs: Vec::new(),
        }
    }

    pub(crate) fn new_connection(
        &mut self,
        connection_id: ConnectionId,
        outgoing: bool,
        audience: Audience,
        now: UnixTimestamp,
        out: &mut HubResults,
    ) {
        let ts = to_ts(now);
        let output = self.engine.handle(Input::NewConnection {
            id: connection_id,
            outgoing,
            audience,
            now: ts,
        });
        self.process_output(output, out);
    }

    pub(crate) fn handle_bytes(
        &mut self,
        connection_id: ConnectionId,
        bytes: Vec<u8>,
        now: UnixTimestamp,
        out: &mut HubResults,
    ) {
        let ts = to_ts(now);
        let output = self.engine.handle(Input::ReceivedBytes {
            id: connection_id,
            bytes,
            now: ts,
        });
        self.process_output(output, out);
    }

    pub(crate) fn connection_lost(&mut self, connection_id: ConnectionId) {
        let _ = self
            .engine
            .handle(Input::ConnectionLost { id: connection_id });
    }

    pub(crate) fn handle_sign_result(
        &mut self,
        task_id: IoTaskId,
        signature: ed25519_dalek::Signature,
        out: &mut HubResults,
    ) {
        let Some(op_id) = self.task_to_op.remove(&task_id) else {
            return;
        };
        let output = self
            .engine
            .handle(Input::SigningComplete { op_id, signature });
        self.process_output(output, out);
    }

    pub(crate) fn handle_storage_result(
        &mut self,
        task_id: IoTaskId,
        result: StorageResult,
        out: &mut HubResults,
    ) {
        let Some(op_id) = self.task_to_op.remove(&task_id) else {
            return;
        };
        let kv_result = storage_result_to_kv(result);
        let output = self.engine.handle(Input::StorageComplete {
            op_id,
            result: kv_result,
        });
        self.process_output(output, out);
    }

    pub(crate) fn find_document(&mut self, document_id: &DocumentId, out: &mut HubResults) {
        let sed_id = document_id.to_sedimentree_id();
        let output = self.engine.handle(Input::FindDocument { sed_id });
        self.process_output(output, out);
    }

    pub(crate) fn on_new_changes_from_doc(
        &mut self,
        document_id: &DocumentId,
        changes: Vec<crate::actors::messages::SubductionChangeInfo>,
        out: &mut HubResults,
    ) {
        let sed_id = document_id.to_sedimentree_id();
        let local_changes: Vec<LocalChange> = changes
            .iter()
            .map(|c| LocalChange {
                parents: c.deps.iter().map(change_hash_to_digest).collect(),
                blob: Blob::new(c.raw_bytes.clone()),
            })
            .collect();
        let output = self.engine.handle(Input::NewLocalChanges {
            sed_id,
            changes: local_changes,
        });
        self.process_output(output, out);
    }

    // ----- Output processing -----

    fn process_output(&mut self, output: EngineOutput<ConnectionId>, hub_out: &mut HubResults) {
        for (conn_id, bytes) in output.send {
            hub_out.emit_io_action(super::io::HubIoAction::Send {
                connection_id: conn_id,
                msg: bytes,
            });
        }

        for issued in output.storage_ops {
            let task_id = hub_out.emit_io_action(super::io::HubIoAction::Storage(
                kv_op_to_storage_task(issued.operation),
            ));
            self.task_to_op.insert(task_id, issued.op_id);
        }

        for req in output.sign_requests {
            let task_id = hub_out.emit_io_action(super::io::HubIoAction::Sign {
                payload_bytes: req.payload_bytes,
            });
            self.task_to_op.insert(task_id, req.op_id);
        }

        for (sed_id, blobs) in output.data_for_docs {
            if let Some(doc_id) = DocumentId::from_sedimentree_id(&sed_id) {
                self.data_for_docs.push(SubductionDataForDoc {
                    document_id: doc_id,
                    blobs,
                    status: SearchStatus::Found,
                });
            }
        }

        for (sed_id, status) in output.search_status {
            if let Some(doc_id) = DocumentId::from_sedimentree_id(&sed_id) {
                // Check if we already have a data_for_docs entry for this doc
                let already_queued = self.data_for_docs.iter().any(|d| d.document_id == doc_id);
                if !already_queued {
                    self.data_for_docs.push(SubductionDataForDoc {
                        document_id: doc_id,
                        blobs: vec![],
                        status,
                    });
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Type conversion helpers
// ---------------------------------------------------------------------------

fn to_ts(now: UnixTimestamp) -> TimestampSeconds {
    TimestampSeconds::new((now.as_millis() / 1000) as u64)
}

fn kv_op_to_storage_task(op: KvOp) -> StorageTask {
    match op {
        KvOp::LoadRange { prefix } => StorageTask::LoadRange {
            prefix: storage_key_convert(&prefix),
        },
        KvOp::Load { key } => StorageTask::Load {
            key: storage_key_convert(&key),
        },
        KvOp::Put { key, value } => StorageTask::Put {
            key: storage_key_convert(&key),
            value,
        },
    }
}

fn storage_key_convert(key: &SubStorageKey) -> crate::StorageKey {
    crate::StorageKey::from_parts(key.components()).expect("valid storage key")
}

fn storage_result_to_kv(result: StorageResult) -> KvResult {
    match result {
        StorageResult::LoadRange { values } => KvResult::LoadRange {
            values: values
                .into_iter()
                .map(|(k, v)| {
                    let components: Vec<String> = k.into_iter().collect();
                    (SubStorageKey::new(components), v)
                })
                .collect(),
        },
        StorageResult::Load { value } => KvResult::Load { value },
        StorageResult::Put | StorageResult::Delete => KvResult::Put,
    }
}

fn change_hash_to_digest(hash: &automerge::ChangeHash) -> Digest<LooseCommit> {
    let hash_str = hash.to_string();
    let bytes: [u8; 32] = hex::decode(&hash_str)
        .ok()
        .and_then(|v| v.try_into().ok())
        .unwrap_or([0u8; 32]);
    Digest::force_from_bytes(bytes)
}
