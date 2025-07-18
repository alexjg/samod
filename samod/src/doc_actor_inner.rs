use automerge::Automerge;
use futures::channel::mpsc;
use samod_core::{
    DocumentActorId, DocumentChanged, DocumentId, UnixTimestamp,
    actors::{
        DocToHubMsg,
        document::{ActorResult, DocumentActor, WithDocResult},
    },
};

use crate::{
    actor_task::ActorTask,
    io_loop::{self, IoLoopTask},
};

pub(crate) struct DocActorInner {
    document_id: DocumentId,
    actor_id: DocumentActorId,
    tx_to_core: mpsc::UnboundedSender<(DocumentActorId, DocToHubMsg)>,
    tx_io: mpsc::UnboundedSender<io_loop::IoLoopTask>,
    ephemera_listeners: Vec<mpsc::UnboundedSender<Vec<u8>>>,
    change_listeners: Vec<mpsc::UnboundedSender<DocumentChanged>>,
    actor: DocumentActor,
}

impl DocActorInner {
    pub(crate) fn new(
        document_id: DocumentId,
        actor_id: DocumentActorId,
        actor: DocumentActor,
        tx_to_core: mpsc::UnboundedSender<(DocumentActorId, DocToHubMsg)>,
        tx_io: mpsc::UnboundedSender<io_loop::IoLoopTask>,
    ) -> Self {
        DocActorInner {
            document_id,
            actor_id,
            tx_to_core,
            tx_io,
            ephemera_listeners: Vec::new(),
            change_listeners: Vec::new(),
            actor,
        }
    }

    pub(crate) fn with_document<F, R>(&mut self, f: F) -> R
    where
        F: FnOnce(&mut Automerge) -> R,
    {
        let WithDocResult {
            actor_result,
            value,
        } = self.actor.with_document(UnixTimestamp::now(), f).unwrap();

        self.handle_results(actor_result);

        value
    }

    pub(crate) fn create_ephemera_listener(&mut self) -> mpsc::UnboundedReceiver<Vec<u8>> {
        let (tx, rx) = mpsc::unbounded();
        self.ephemera_listeners.push(tx);
        rx
    }

    pub(crate) fn create_change_listener(&mut self) -> mpsc::UnboundedReceiver<DocumentChanged> {
        let (tx, rx) = mpsc::unbounded();
        self.change_listeners.push(tx);
        rx
    }

    pub(crate) fn broadcast_ephemeral_message(&mut self, message: Vec<u8>) {
        let result = self.actor.broadcast(UnixTimestamp::now(), message);
        self.handle_results(result);
    }

    pub(crate) fn handle_results(&mut self, results: ActorResult) {
        let ActorResult {
            io_tasks,
            outgoing_messages,
            ephemeral_messages,
            change_events,
        } = results;
        for task in io_tasks {
            let _ = self
                .tx_io
                // .unbounded_send((task.id, Some(self.actor_id), storage_task));
                .unbounded_send(IoLoopTask {
                    doc_id: self.document_id.clone(),
                    task,
                    actor_id: self.actor_id,
                });
        }

        for msg in outgoing_messages {
            let _ = self.tx_to_core.unbounded_send((self.actor_id, msg));
        }

        if !ephemeral_messages.is_empty() {
            self.ephemera_listeners.retain_mut(|listener| {
                for msg in &ephemeral_messages {
                    if listener.unbounded_send(msg.clone()).is_err() {
                        return false;
                    }
                }
                true
            });
        }

        if !change_events.is_empty() {
            self.change_listeners.retain_mut(|listener| {
                for change in &change_events {
                    if listener.unbounded_send(change.clone()).is_err() {
                        return false;
                    }
                }
                true
            });
        }
    }

    pub(crate) fn handle_task(&mut self, task: ActorTask) {
        let result = match task {
            ActorTask::HandleMessage(samod_to_actor_message) => self
                .actor
                .handle_message(UnixTimestamp::now(), samod_to_actor_message),
            ActorTask::IoComplete(io_result) => self
                .actor
                .handle_io_complete(UnixTimestamp::now(), io_result),
        };
        self.handle_results(result.unwrap());
    }
}
