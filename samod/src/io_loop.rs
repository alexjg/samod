use std::sync::{Arc, Mutex};

use futures::{FutureExt, StreamExt, stream::FuturesUnordered};
use samod_core::{
    DocumentActorId, DocumentId, PeerId,
    actors::document::io::{DocumentIoResult, DocumentIoTask},
    io::{IoResult, IoTask, StorageResult, StorageTask},
};

use crate::{
    ActorHandle, Inner, actor_task::ActorTask, announce_policy::LocalAnnouncePolicy,
    storage::LocalStorage, unbounded::UnboundedReceiver,
};

pub(crate) struct IoLoopTask {
    pub(crate) doc_id: DocumentId,
    pub(crate) task: IoTask<DocumentIoTask>,
    pub(crate) actor_id: DocumentActorId,
}

struct IoLoopResult {
    result: IoResult<DocumentIoResult>,
    actor_id: DocumentActorId,
}

#[tracing::instrument(skip(inner, storage, announce_policy, rx))]
pub(crate) async fn io_loop<S: LocalStorage, A: LocalAnnouncePolicy>(
    local_peer_id: PeerId,
    inner: Arc<Mutex<Inner>>,
    storage: S,
    announce_policy: A,
    rx: UnboundedReceiver<IoLoopTask>,
) {
    let mut running_tasks = FuturesUnordered::new();

    loop {
        futures::select! {
            next_task = rx.recv().fuse() => {
                let Some(next_task) = next_task.ok() else {
                    tracing::trace!("storage loop channel closed, exiting");
                    break;
                };
                running_tasks.push({
                    let storage = storage.clone();
                    let announce_policy = announce_policy.clone();
                    async move {
                        let IoLoopTask { doc_id, task, actor_id } = next_task;
                        let result = dispatch_document_task(storage.clone(), announce_policy.clone(), doc_id.clone(), task).await;
                        IoLoopResult {
                            result,
                            actor_id,
                        }
                    }
                });
            }
            result = running_tasks.select_next_some() => {
                let IoLoopResult { actor_id, result } = result;
                let inner = inner.lock().unwrap();
                let Some(ActorHandle{tx,.. }) = inner.actors.get(&actor_id) else {
                    tracing::warn!(?actor_id, "received io result for unknown actor");
                    continue;
                };
                let _ = tx.unbounded_send(ActorTask::IoComplete(result));
            }
        }
    }

    while let Some(IoLoopResult { result, actor_id }) = running_tasks.next().await {
        let inner = inner.lock().unwrap();
        let Some(ActorHandle { tx, .. }) = inner.actors.get(&actor_id) else {
            tracing::warn!(?actor_id, "received io result for unknown actor");
            continue;
        };
        let _ = tx.unbounded_send(ActorTask::IoComplete(result));
    }
}

async fn dispatch_document_task<S: LocalStorage, A: LocalAnnouncePolicy>(
    storage: S,
    announce: A,
    document_id: DocumentId,
    task: IoTask<DocumentIoTask>,
) -> IoResult<DocumentIoResult> {
    match task.action {
        DocumentIoTask::Storage(storage_task) => IoResult {
            task_id: task.task_id,
            payload: DocumentIoResult::Storage(dispatch_storage_task(storage_task, storage).await),
        },
        DocumentIoTask::CheckAnnouncePolicy { peer_id } => IoResult {
            task_id: task.task_id,
            payload: DocumentIoResult::CheckAnnouncePolicy(
                announce.should_announce(document_id, peer_id).await,
            ),
        },
    }
}

#[tracing::instrument(skip(task, storage))]
pub(crate) async fn dispatch_storage_task<S: LocalStorage>(
    task: StorageTask,
    storage: S,
) -> StorageResult {
    match task {
        StorageTask::Load { key } => {
            tracing::trace!(?key, "loading key from storage");
            let value = storage.load(key).await;
            StorageResult::Load { value }
        }
        StorageTask::LoadRange { prefix } => {
            tracing::trace!(?prefix, "loading range from storage");
            let values = storage.load_range(prefix).await;
            StorageResult::LoadRange { values }
        }
        StorageTask::Put { key, value } => {
            tracing::trace!(?key, "putting value into storage");
            storage.put(key, value).await;
            StorageResult::Put
        }
        StorageTask::Delete { key } => {
            tracing::trace!(?key, "deleting key from storage");
            storage.delete(key).await;
            StorageResult::Delete
        }
    }
}
