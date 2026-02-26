use std::sync::{Arc, Mutex};

use futures::{
    FutureExt, SinkExt, StreamExt,
    stream::{BoxStream, FuturesUnordered},
};
use samod_core::{
    DialerId, DocumentActorId, DocumentId, PeerId,
    actors::{
        document::io::{DocumentIoResult, DocumentIoTask},
        hub::{CommandResult, DispatchedCommand, HubEvent},
    },
    io::{IoResult, IoTask, StorageResult, StorageTask},
};
use url::Url;

use crate::{
    ConnFinishedReason, Inner,
    actor_task::ActorTask,
    announce_policy::LocalAnnouncePolicy,
    connection::ConnectionHandle,
    observer::{self, RepoObserver},
    storage::LocalStorage,
    transport::BoxSink,
    unbounded::UnboundedReceiver,
};

#[derive(Debug)]
pub(crate) enum IoLoopTask {
    DriveConnection(DriveConnectionTask),
    Storage {
        doc_id: DocumentId,
        task: IoTask<DocumentIoTask>,
        actor_id: DocumentActorId,
    },
    EstablishTransport {
        dialer_id: DialerId,
        url: Url,
    },
    /// Periodic tick to drive time-based connector logic (retry backoff).
    Tick,
}

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

/// Type alias for a shared, type-erased dialer.
pub(crate) type DynDialer = Arc<dyn crate::Dialer>;

#[tracing::instrument(skip(inner, storage, announce_policy, rx, dialers, observer))]
pub(crate) async fn io_loop<S: LocalStorage, A: LocalAnnouncePolicy>(
    local_peer_id: PeerId,
    inner: Arc<Mutex<Inner>>,
    storage: S,
    announce_policy: A,
    rx: UnboundedReceiver<IoLoopTask>,
    dialers: Arc<Mutex<std::collections::HashMap<DialerId, DynDialer>>>,
    observer: Option<Arc<dyn RepoObserver>>,
) {
    let mut running_storage_tasks = FuturesUnordered::new();
    let mut running_connections = FuturesUnordered::new();
    let mut running_transport_establishments = FuturesUnordered::new();

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
                            let observer = observer.clone();
                            async move {
                                let result = dispatch_document_task(storage.clone(), announce_policy.clone(), doc_id.clone(), task, observer.as_deref()).await;
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
                    IoLoopTask::EstablishTransport { dialer_id, url } => {
                        let dialer = {
                            let dialers = dialers.lock().unwrap();
                            dialers.get(&dialer_id).cloned()
                        };
                        if let Some(dialer) = dialer {
                            let inner = inner.clone();
                            let url_clone = url.clone();
                            let observer = observer.clone();
                            running_transport_establishments.push(async move {
                                establish_transport(inner, dialer_id, url_clone, dialer, observer.as_deref()).await
                            });
                        } else {
                            tracing::warn!(
                                ?dialer_id,
                                %url,
                                "no dialer registered"
                            );
                            let event = HubEvent::dial_failed(
                                dialer_id,
                                "no dialer registered".to_string(),
                            );
                            inner.lock().unwrap().handle_event(event);
                        }
                    }
                    IoLoopTask::Tick => {
                        inner.lock().unwrap().handle_event(HubEvent::tick());
                    }
                }
            }
            result = running_storage_tasks.select_next_some() => {
                let StorageTaskComplete { actor_id, result } = result;
                let inner = inner.lock().unwrap();
                inner.dispatch_task(actor_id, ActorTask::IoComplete(result));
            },
            _ = running_connections.select_next_some() => {

            },
            _ = running_transport_establishments.select_next_some() => {
                // Transport establishment results are handled inside the future itself
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
    obs: Option<&dyn RepoObserver>,
) -> IoResult<DocumentIoResult> {
    match task.action {
        DocumentIoTask::Storage(storage_task) => {
            let operation = match &storage_task {
                StorageTask::Load { .. } => observer::StorageOperation::Load,
                StorageTask::LoadRange { .. } => observer::StorageOperation::LoadRange,
                StorageTask::Put { .. } => observer::StorageOperation::Put,
                StorageTask::Delete { .. } => observer::StorageOperation::Delete,
            };
            let start = std::time::Instant::now();
            let result = dispatch_storage_task(storage_task, storage).await;
            if let Some(obs) = obs {
                obs.observe(&observer::RepoEvent::StorageOperationCompleted {
                    operation,
                    duration: start.elapsed(),
                });
            }
            IoResult {
                task_id: task.task_id,
                payload: DocumentIoResult::Storage(result),
            }
        }
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

/// Attempt to establish a transport for a dialer.
///
/// Calls the dialer, and on success creates a connection in the hub
/// atomically associated with the dialer. On failure, notifies the hub so
/// backoff can be applied.
///
/// The `url` parameter is used for logging only — the dialer knows its own URL.
async fn establish_transport(
    inner: Arc<Mutex<Inner>>,
    dialer_id: DialerId,
    url: Url,
    dialer: DynDialer,
    obs: Option<&dyn RepoObserver>,
) {
    tracing::debug!(?dialer_id, %url, "establishing transport");

    match dialer.connect().await {
        Ok(transport) => {
            tracing::debug!(?dialer_id, %url, "transport established successfully");

            // Create a connection atomically associated with the dialer
            let conn_handle = {
                let mut inner_guard = inner.lock().unwrap();
                let DispatchedCommand { command_id, event } =
                    HubEvent::create_dialer_connection(dialer_id);

                let (tx_result, mut rx_result) = futures::channel::oneshot::channel();
                inner_guard.pending_commands.insert(command_id, tx_result);
                inner_guard.handle_event(event);

                let connection_id = match rx_result.try_recv() {
                    Ok(Some(CommandResult::CreateConnection { connection_id })) => connection_id,
                    other => {
                        tracing::error!(
                            ?dialer_id,
                            ?other,
                            "unexpected result from create_dialer_connection"
                        );
                        let event = HubEvent::dial_failed(
                            dialer_id,
                            "internal error creating connection".to_string(),
                        );
                        inner_guard.handle_event(event);
                        return;
                    }
                };

                if let Some(obs) = obs {
                    obs.observe(&observer::RepoEvent::ConnectionEstablished { connection_id });
                }

                inner_guard
                    .connections
                    .get(&connection_id)
                    .expect("connection should exist after create_dialer_connection")
                    .clone()
            };

            // Wire up the connection to the IO loop
            let drive_task = DriveConnectionTask {
                conn_handle,
                stream: transport.stream,
                sink: transport.sink,
            };

            // Drive the connection directly — we're already in the IO loop
            drive_connection(inner, drive_task).await;
        }
        Err(e) => {
            tracing::warn!(
                ?dialer_id,
                %url,
                error = %e,
                "dial failed"
            );
            let event = HubEvent::dial_failed(dialer_id, e.to_string());
            inner.lock().unwrap().handle_event(event);
        }
    }
}
