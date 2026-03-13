use std::collections::{HashMap, HashSet, VecDeque};

use automerge::Automerge;
use samod_core::{
    ConnectionId, DocumentActorId, DocumentChanged, DocumentId, PeerId, UnixTimestamp,
    actors::{
        DocToHubMsg, HubToDocMsg,
        document::{
            DocActorResult, DocumentActor, DocumentError, SpawnArgs, WithDocResult,
            io::{DocumentIoResult, DocumentIoTask},
        },
    },
    io::{IoResult, IoTask, IoTaskId},
    network::PeerDocState,
};

use crate::Storage;

pub(crate) struct DocActorRunner {
    #[allow(dead_code)]
    id: DocumentActorId,
    doc_id: DocumentId,
    actor: DocumentActor,
    inbox: VecDeque<ActorEvent>,
    outbox: VecDeque<DocToHubMsg>,
    ephemera: Vec<Vec<u8>>,
    doc_changed: Vec<DocumentChanged>,
    peer_doc_state_changes: Vec<HashMap<ConnectionId, PeerDocState>>,
    pending_storage_tasks: HashSet<IoTaskId>,
}

impl DocActorRunner {
    pub(crate) fn new(now: UnixTimestamp, args: SpawnArgs) -> Self {
        let id = args.actor_id();
        let doc_id = args.document_id().clone();
        let (actor, results) = DocumentActor::new(now, args);
        let mut runner = DocActorRunner {
            id,
            doc_id,
            actor,
            inbox: VecDeque::new(),
            outbox: VecDeque::new(),
            ephemera: Vec::new(),
            doc_changed: Vec::new(),
            peer_doc_state_changes: Vec::new(),
            pending_storage_tasks: HashSet::new(),
        };
        runner.enqueue_events(results);
        runner
    }

    pub(crate) fn handle_events(
        &mut self,
        now: UnixTimestamp,
        storage: &mut Storage,
        announce_policy: &dyn Fn(DocumentId, PeerId) -> bool,
    ) {
        self.handle_completed_storage(now, storage);
        while let Some(event) = self.inbox.pop_front() {
            if self.actor.is_stopped() {
                self.inbox.clear();
                return;
            }
            match event {
                ActorEvent::Message(msg) => {
                    let result = self
                        .actor
                        .handle_message(now, msg)
                        .expect("failed to handle actor message");
                    self.enqueue_events(result);
                }
                ActorEvent::Io(task) => match task.action {
                    DocumentIoTask::Storage(storage_task) => {
                        storage.handle_task(task.task_id, storage_task);
                        self.pending_storage_tasks.insert(task.task_id);
                    }
                    DocumentIoTask::CheckAnnouncePolicy { peer_id } => {
                        let io_result = IoResult {
                            task_id: task.task_id,
                            payload: DocumentIoResult::CheckAnnouncePolicy(announce_policy(
                                self.doc_id.clone(),
                                peer_id,
                            )),
                        };
                        let actor_result = self
                            .actor
                            .handle_io_complete(now, io_result)
                            .expect("failed to handle IO completion");
                        self.enqueue_events(actor_result);
                    }
                },
            }
            self.handle_completed_storage(now, storage);
        }
    }

    fn handle_completed_storage(&mut self, now: UnixTimestamp, storage: &mut Storage) {
        let mut completed = Vec::new();
        self.pending_storage_tasks.retain(|task_id| {
            let Some(completed_task) = storage.check_pending_task(*task_id) else {
                return true;
            };
            completed.push((*task_id, completed_task));
            false
        });
        for (task_id, completed_task) in completed {
            let io_result = IoResult {
                task_id,
                payload: DocumentIoResult::Storage(completed_task),
            };
            let actor_result = self
                .actor
                .handle_io_complete(now, io_result)
                .expect("failed to handle IO completion");
            self.enqueue_events(actor_result);
        }
    }

    fn enqueue_events(&mut self, result: DocActorResult) {
        let DocActorResult {
            io_tasks,
            outgoing_messages,
            ephemeral_messages,
            change_events,
            stopped: _,
            peer_state_changes,
            sync_message_stats: _,
            pending_sync_messages: _,
        } = result;
        for task in io_tasks {
            self.inbox.push_back(ActorEvent::Io(task));
        }
        for msg in outgoing_messages {
            self.outbox.push_back(msg);
        }
        self.ephemera.extend(ephemeral_messages);
        self.doc_changed.extend(change_events);
        if !peer_state_changes.is_empty() {
            self.peer_doc_state_changes.push(peer_state_changes);
        }
    }

    pub fn with_document<F, R>(&mut self, now: UnixTimestamp, f: F) -> Result<R, DocumentError>
    where
        F: FnOnce(&mut Automerge) -> R,
    {
        let WithDocResult {
            actor_result,
            value,
        } = self.actor.with_document(now, f)?;
        self.enqueue_events(actor_result);
        Ok(value)
    }

    pub fn broadcast(&mut self, now: UnixTimestamp, msg: Vec<u8>) {
        let result = self.actor.broadcast(now, msg);
        self.enqueue_events(result);
    }

    pub(crate) fn deliver_message_to_inbox(&mut self, message: HubToDocMsg) {
        self.inbox.push_back(ActorEvent::Message(message));
    }

    pub(crate) fn take_outbox(&mut self) -> Vec<DocToHubMsg> {
        self.outbox.drain(..).collect()
    }

    pub(crate) fn actor(&self) -> &DocumentActor {
        &self.actor
    }

    pub(crate) fn document_id(&self) -> &DocumentId {
        &self.doc_id
    }

    pub(crate) fn pop_ephemera(&mut self) -> Vec<Vec<u8>> {
        std::mem::take(&mut self.ephemera)
    }

    pub(crate) fn pop_doc_changed(&mut self) -> Vec<DocumentChanged> {
        std::mem::take(&mut self.doc_changed)
    }

    pub(crate) fn is_stopped(&self) -> bool {
        self.actor.is_stopped()
    }

    pub(crate) fn peer_doc_state_changes(&self) -> &[HashMap<ConnectionId, PeerDocState>] {
        &self.peer_doc_state_changes
    }

    #[expect(dead_code)]
    pub(crate) fn pop_peer_doc_state_changes(
        &mut self,
    ) -> Vec<HashMap<ConnectionId, PeerDocState>> {
        std::mem::take(&mut self.peer_doc_state_changes)
    }
}

enum ActorEvent {
    Message(HubToDocMsg),
    Io(IoTask<DocumentIoTask>),
}
