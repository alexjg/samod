use std::collections::HashMap;

use automerge::ChangeHash;

use crate::{
    ConnectionId, DocumentChanged, DocumentId, PeerId, StorageKey,
    actors::{
        DocToHubMsg,
        document::{DocumentStatus, SyncMessageStat, io::DocumentIoTask},
        messages::{Broadcast, DocToHubMsgPayload, SyncMessage},
    },
    io::{IoTask, IoTaskId, StorageTask},
    network::PeerDocState,
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
    /// Connections which have changed state for this document
    pub peer_state_changes: HashMap<ConnectionId, PeerDocState>,
    /// Sync message statistics for observability
    pub sync_message_stats: Vec<SyncMessageStat>,
    /// Number of pending sync messages queued during Loading phase
    pub pending_sync_messages: usize,
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
            peer_state_changes: HashMap::new(),
            sync_message_stats: Vec::new(),
            pending_sync_messages: 0,
        }
    }

    pub(crate) fn emit_ephemeral_message(&mut self, msg: Vec<u8>) {
        self.ephemeral_messages.push(msg);
    }

    pub(crate) fn emit_doc_changed(&mut self, new_heads: Vec<ChangeHash>) {
        self.change_events.push(DocumentChanged { new_heads });
    }

    /// Send a message back to the hub
    pub(crate) fn send_sync_message(
        &mut self,
        conn_id: ConnectionId,
        doc_id: DocumentId,
        message: SyncMessage,
    ) {
        self.outgoing_messages
            .push(DocToHubMsg(DocToHubMsgPayload::SendSyncMessage {
                connection_id: conn_id,
                document_id: doc_id,
                message,
            }));
    }

    pub(crate) fn send_broadcast(&mut self, connections: Vec<ConnectionId>, msg: Broadcast) {
        self.outgoing_messages
            .push(DocToHubMsg(DocToHubMsgPayload::Broadcast {
                connections,
                msg,
            }));
    }

    pub(crate) fn send_terminated(&mut self) {
        self.outgoing_messages
            .push(DocToHubMsg(DocToHubMsgPayload::Terminated));
    }

    pub(crate) fn send_peer_states_changes(
        &mut self,
        new_states: HashMap<ConnectionId, PeerDocState>,
    ) {
        // Remove previous peer state change messages as they are redundant
        self.outgoing_messages
            .retain(|m| !matches!(m.0, DocToHubMsgPayload::PeerStatesChanged { .. }));
        self.outgoing_messages
            .push(DocToHubMsg(DocToHubMsgPayload::PeerStatesChanged {
                new_states,
            }));
    }

    pub(crate) fn send_doc_status_update(&mut self, new_status: DocumentStatus) {
        // remove any existing doc status update so that if the document status changes
        // multiple times during a turn, only the latest status is sent to the hub.
        // This is especially important to avoid bouncing through a NotFound state
        // when loading a document as that will cause any outstanding find commands
        // to fail even if the document loads successfully in this turn (as it might
        // if we finish loading after receiving a sync message with the document
        // content).
        self.outgoing_messages
            .retain(|m| !matches!(m.0, DocToHubMsgPayload::DocumentStatusChanged { .. }));
        self.outgoing_messages
            .push(DocToHubMsg(DocToHubMsgPayload::DocumentStatusChanged {
                new_status,
            }));
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
