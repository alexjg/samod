use std::collections::HashMap;

use automerge::ChangeHash;

use crate::{
    PeerId, StorageKey,
    actors::{driver::ActorIo, messages::DocToHubMsgPayload},
    io::{StorageResult, StorageTask},
};

use super::{
    DocumentActor,
    io::{DocumentIoResult, DocumentIoTask},
    run::ActorOutput,
};

/// Helper struct for managing IO operations within the async runtime.
///
/// This provides a clean interface for async operations to request IO
/// and await their completion.
#[derive(Clone)]
pub(super) struct ActorIoAccess {
    io: ActorIo<DocumentActor>,
}

impl ActorIoAccess {
    pub fn new(io: ActorIo<DocumentActor>) -> Self {
        Self { io }
    }

    /// Load a single value from storage
    #[allow(dead_code)]
    pub async fn load(&self, key: StorageKey) -> Option<Vec<u8>> {
        let task = DocumentIoTask::Storage(StorageTask::Load { key });
        let result = self.dispatch_task(task).await;

        match result {
            DocumentIoResult::Storage(StorageResult::Load { value }) => value,
            _ => panic!("Expected Load result, got {result:?}"),
        }
    }

    /// Load a range of values from storage
    pub async fn load_range(&self, prefix: StorageKey) -> HashMap<StorageKey, Vec<u8>> {
        let task = DocumentIoTask::Storage(StorageTask::LoadRange { prefix });
        let result = self.dispatch_task(task).await;

        match result {
            DocumentIoResult::Storage(StorageResult::LoadRange { values }) => values,
            _ => panic!("Expected LoadRange result, got {result:?}"),
        }
    }

    /// Store a value in storage
    pub async fn put(&self, key: StorageKey, value: Vec<u8>) {
        let task = DocumentIoTask::Storage(StorageTask::Put { key, value });
        let result = self.dispatch_task(task).await;

        match result {
            DocumentIoResult::Storage(StorageResult::Put) => (),
            _ => panic!("Expected Put result, got {result:?}"),
        }
    }

    /// Delete a value from storage
    pub async fn delete(&self, key: StorageKey) {
        let result = self
            .dispatch_task(DocumentIoTask::Storage(StorageTask::Delete { key }))
            .await;

        match result {
            DocumentIoResult::Storage(StorageResult::Delete) => (),
            _ => panic!("Expected Delete result, got {result:?}"),
        }
    }

    pub(crate) async fn check_announce_policy(&self, peer_id: PeerId) -> bool {
        let result = self
            .dispatch_task(DocumentIoTask::CheckAnnouncePolicy { peer_id })
            .await;

        match result {
            DocumentIoResult::CheckAnnouncePolicy(result) => result,
            _ => panic!("Expected CheckAnnouncePolicy result, got {result:?}"),
        }
    }

    pub fn emit_ephemeral_message(&self, msg: Vec<u8>) {
        self.io.emit_event(ActorOutput::EphemeralMessage(msg));
    }

    pub fn emit_doc_changed(&self, new_heads: Vec<ChangeHash>) {
        self.io.emit_event(ActorOutput::DocChanged { new_heads });
    }

    /// Send a message back to the hub
    pub fn send_message(&self, message: DocToHubMsgPayload) {
        self.io.emit_event(ActorOutput::Message(message));
    }

    /// Dispatch a document IO task and await its completion
    async fn dispatch_task(&self, task: DocumentIoTask) -> DocumentIoResult {
        self.io.perform_io(task).await
    }
}
