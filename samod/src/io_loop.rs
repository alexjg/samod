use std::{
    pin::Pin,
    sync::{Arc, Mutex},
};

use futures::{
    FutureExt, Sink, SinkExt, StreamExt,
    stream::{BoxStream, FuturesUnordered},
};
use samod_core::{
    DocumentActorId, DocumentId, PeerId,
    actors::{
        document::io::{DocumentIoResult, DocumentIoTask},
        hub::{DispatchedCommand, HubEvent},
    },
    io::{IoResult, IoTask, StorageResult, StorageTask},
};

use crate::{
    ConnFinishedReason, Inner, actor_task::ActorTask, announce_policy::LocalAnnouncePolicy,
    connection::ConnectionHandle, storage::LocalStorage, unbounded::UnboundedReceiver,
};

#[derive(Debug)]
pub(crate) enum IoLoopTask {
    DriveConnection(DriveConnectionTask),
    Storage {
        doc_id: DocumentId,
        task: IoTask<DocumentIoTask>,
        actor_id: DocumentActorId,
    },
}

type BoxSink<T> = Pin<
    Box<
        dyn Sink<T, Error = Box<dyn std::error::Error + Send + Sync + 'static>>
            + Send
            + 'static
            + Unpin,
    >,
>;

pub(crate) struct DriveConnectionTask {
    pub(crate) conn_handle: ConnectionHandle,
    pub(crate) stream:
        BoxStream<'static, Result<Vec<u8>, Box<dyn std::error::Error + Send + Sync + 'static>>>,
    pub(crate) sink: BoxSink<Vec<u8>>,
}

impl std::fmt::Debug for DriveConnectionTask {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DriveConnectionTask")
            .field("connection_id", &self.conn_handle.id())
            .finish()
    }
}

struct StorageTaskComplete {
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
    let mut running_storage_tasks = FuturesUnordered::new();
    let mut running_connections = FuturesUnordered::new();

    loop {
        futures::select! {
            next_task = rx.recv().fuse() => {
                let Some(next_task) = next_task.ok() else {
                    tracing::trace!("storage loop channel closed, exiting");
                    break;
                };
                tracing::trace!("received task");
                match next_task {
                    IoLoopTask::Storage { doc_id, task, actor_id } => {
                        running_storage_tasks.push({
                            let storage = storage.clone();
                            let announce_policy = announce_policy.clone();
                            async move {
                                // let IoLoopTask { doc_id, task, actor_id } = next_task;
                                let result = dispatch_document_task(storage.clone(), announce_policy.clone(), doc_id.clone(), task).await;
                                StorageTaskComplete {
                                    result,
                                    actor_id,
                                }
                            }
                        });
                    },
                    IoLoopTask::DriveConnection(task) => {
                        running_connections.push(drive_connection(inner.clone(), task));
                    }
                }
            }
            result = running_storage_tasks.select_next_some() => {
                let StorageTaskComplete { actor_id, result } = result;
                let inner = inner.lock().unwrap();
                inner.dispatch_task(actor_id, ActorTask::IoComplete(result));
            },
            _ = running_connections.select_next_some() => {

            }
        }
    }

    while let Some(StorageTaskComplete { result, actor_id }) = running_storage_tasks.next().await {
        let inner = inner.lock().unwrap();
        inner.dispatch_task(actor_id, ActorTask::IoComplete(result));
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

async fn drive_connection(
    inner: Arc<Mutex<Inner>>,
    DriveConnectionTask {
        conn_handle,
        stream,
        mut sink,
    }: DriveConnectionTask,
) {
    let rx = conn_handle.take_rx();
    let connection_id = conn_handle.id();
    let mut stream = stream.fuse();
    let result = loop {
        futures::select! {
            next_inbound_msg = stream.next() => {
                if let Some(msg) = next_inbound_msg {
                    match msg {
                        Ok(msg) => {
                            let DispatchedCommand { event, .. } = HubEvent::receive(connection_id, msg);
                            inner.lock().unwrap().handle_event(event);
                        }
                        Err(e) => {
                            tracing::error!(err=?e, "error receiving, closing connection");
                            break ConnFinishedReason::ErrorReceiving(e.to_string());
                        }
                    }
                } else {
                    tracing::debug!("stream closed, closing connection");
                    break ConnFinishedReason::TheyDisconnected;
                }
            },
            next_outbound = rx.recv().fuse() => {
                if let Ok(next_outbound) = next_outbound {
                    if let Err(e) = sink.send(next_outbound).await {
                        tracing::error!(err=?e, "error sending, closing connection");
                        break ConnFinishedReason::ErrorSending(e.to_string());
                    }
                } else {
                    tracing::debug!(?connection_id, "connection closing");
                    break ConnFinishedReason::WeDisconnected;
                }
            }
        }
    };
    if !(result == ConnFinishedReason::WeDisconnected) {
        let event = HubEvent::connection_lost(connection_id);
        inner.lock().unwrap().handle_event(event);
    }
    if let Err(e) = sink.close().await {
        tracing::error!(err=?e, "error closing sink");
    }
    conn_handle.notify_finished(result);
}
