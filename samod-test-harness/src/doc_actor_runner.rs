use std::collections::VecDeque;

use automerge::Automerge;
use samod_core::{
    DocumentActorId, DocumentChanged, DocumentId, PeerId, UnixTimestamp,
    actors::{
        DocToHubMsg, HubToDocMsg,
        document::{
            DocActorResult, DocumentActor, DocumentError, SpawnArgs, WithDocResult,
            io::{DocumentIoResult, DocumentIoTask},
        },
    },
    io::{IoResult, IoTask},
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
                ActorEvent::Io(task) => {
                    let io_result = match task.action {
                        DocumentIoTask::Storage(storage_task) => IoResult {
                            task_id: task.task_id,
                            payload: DocumentIoResult::Storage(storage.handle_task(storage_task)),
                        },
                        DocumentIoTask::CheckAnnouncePolicy { peer_id } => IoResult {
                            task_id: task.task_id,
                            payload: DocumentIoResult::CheckAnnouncePolicy(announce_policy(
                                self.doc_id.clone(),
                                peer_id,
                            )),
                        },
                    };
                    let actor_result = self
                        .actor
                        .handle_io_complete(now, io_result)
                        .expect("failed to handle IO completion");
                    self.enqueue_events(actor_result);
                }
            }
        }
    }

    fn enqueue_events(&mut self, result: DocActorResult) {
        let DocActorResult {
            io_tasks,
            outgoing_messages,
            ephemeral_messages,
            change_events,
            stopped: _,
        } = result;
        for task in io_tasks {
            self.inbox.push_back(ActorEvent::Io(task));
        }
        for msg in outgoing_messages {
            self.outbox.push_back(msg);
        }
        self.ephemera.extend(ephemeral_messages);
        self.doc_changed.extend(change_events);
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
}

enum ActorEvent {
    Message(HubToDocMsg),
    Io(IoTask<DocumentIoTask>),
}
