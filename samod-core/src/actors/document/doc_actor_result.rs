use automerge::ChangeHash;

use crate::{
    DocumentChanged, PeerId, StorageKey,
    actors::{DocToHubMsg, document::io::DocumentIoTask, messages::DocToHubMsgPayload},
    io::{IoTask, IoTaskId, StorageTask},
};

/// Result of processing a message or I/O completion.
#[derive(Debug)]
pub struct DocActorResult {
    /// Document I/O tasks that need to be executed by the caller.
    pub io_tasks: Vec<IoTask<DocumentIoTask>>,
    /// Messages to send back to the main system.
    pub outgoing_messages: Vec<DocToHubMsg>,
    /// New ephemeral messages
    pub ephemeral_messages: Vec<Vec<u8>>,
    /// Change events
    pub change_events: Vec<DocumentChanged>,
    /// Whether this document actor is stopped
    pub stopped: bool,
}

impl DocActorResult {
    /// Creates an empty result.
    pub fn new() -> Self {
        Self {
            io_tasks: Vec::new(),
            outgoing_messages: Vec::new(),
            ephemeral_messages: Vec::new(),
            change_events: Vec::new(),
            stopped: false,
        }
    }

    pub(crate) fn emit_ephemeral_message(&mut self, msg: Vec<u8>) {
        self.ephemeral_messages.push(msg);
    }

    pub(crate) fn emit_doc_changed(&mut self, new_heads: Vec<ChangeHash>) {
        self.change_events.push(DocumentChanged { new_heads });
    }

    /// Send a message back to the hub
    pub(crate) fn send_message(&mut self, message: DocToHubMsgPayload) {
        self.outgoing_messages.push(DocToHubMsg(message));
    }

    pub(crate) fn put(&mut self, key: StorageKey, value: Vec<u8>) -> IoTaskId {
        self.enqueue_task(DocumentIoTask::Storage(StorageTask::Put { key, value }))
    }

    pub(crate) fn delete(&mut self, key: StorageKey) -> IoTaskId {
        self.enqueue_task(DocumentIoTask::Storage(StorageTask::Delete { key }))
    }

    pub(crate) fn check_announce_policy(&mut self, peer_id: PeerId) -> IoTaskId {
        self.enqueue_task(DocumentIoTask::CheckAnnouncePolicy { peer_id })
    }

    fn enqueue_task(&mut self, task: DocumentIoTask) -> IoTaskId {
        let io_task = IoTask::new(task);
        let task_id = io_task.task_id;
        self.io_tasks.push(io_task);
        task_id
    }
}

impl Default for DocActorResult {
    fn default() -> Self {
        Self::new()
    }
}
