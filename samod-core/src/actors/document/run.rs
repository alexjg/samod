use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};

use futures::{FutureExt, StreamExt, channel::mpsc, future::BoxFuture, stream::FuturesUnordered};

use crate::{
    ConnectionId, PeerId, StorageKey, UnixTimestamp,
    actors::{
        RunState,
        document::DocumentStatus,
        driver::ActorIo,
        messages::{DocMessage, DocToHubMsgPayload},
    },
};

use super::{
    DocumentActor, DocumentError,
    compaction::{Job, JobComplete},
    peer_doc_connection::AnnouncePolicy,
};

mod actor_input;
mod actor_output;

pub(super) use actor_input::ActorInput;
pub(super) use actor_output::ActorOutput;

use super::{ActorIoAccess, ActorState};

/// The main async runtime loop for the document actor.
pub async fn actor_run(
    now: Arc<Mutex<UnixTimestamp>>,
    mut rx_input: mpsc::UnboundedReceiver<ActorInput>,
    io: ActorIo<DocumentActor>,
    state: Arc<Mutex<ActorState>>,
    initial_connections: HashMap<ConnectionId, (PeerId, Option<DocMessage>)>,
) {
    tracing::trace!(?initial_connections, "applying initial connections");
    let io = ActorIoAccess::new(io);
    for (conn_id, (peer_id, msg)) in initial_connections {
        state.lock().unwrap().add_connection(conn_id, peer_id);
        if let Some(msg) = msg {
            state.lock().unwrap().handle_doc_message(
                *now.lock().unwrap(),
                io.clone(),
                conn_id,
                msg,
            );
        }
    }

    // Collection of futures representing operations that are currently executing
    let mut running_operations = FuturesUnordered::new();

    if state.lock().unwrap().is_loading() {
        tracing::trace!("starting new loading doc actor");
        running_operations.push(load(io.clone(), state.clone(), now.clone()).boxed());
    } else {
        tracing::debug!("document actor is ready immediately");
        io.send_message(DocToHubMsgPayload::DocumentStatusChanged {
            new_status: DocumentStatus::Ready,
        });
    }

    loop {
        if state.lock().unwrap().run_state() == RunState::Stopping {
            tracing::debug!("Document actor is stopping, exiting runtime loop");
            // Exit the runtime loop if the actor is stopping
            break;
        }
        enqueue_announce_policy_checks(io.clone(), state.clone(), &mut running_operations);
        generate_sync_messages(io.clone(), state.clone(), now.clone());
        save_new_changes(io.clone(), state.clone(), &mut running_operations);

        futures::select! {
            input = rx_input.select_next_some() => {
                if matches!(input, ActorInput::Terminate) {
                    tracing::debug!("terminating document actor");
                    state.lock().unwrap().set_run_state(RunState::Stopping);
                    // Exit the runtime loop after receiving a termination signal
                    break;
                }
                let state_clone = state.clone();
                let io_clone = io.clone();
                let operation_future = handle_input(now.clone(), state_clone, io_clone, input);
                running_operations.push(operation_future.boxed());
            }
            operation_result = running_operations.select_next_some() => {
                // Handle completed operation
                if let Err(e) = operation_result {
                    tracing::error!("Document actor operation failed: {:?}", e);
                }
            },
        }
    }

    // Wait for all operations to complete before exiting
    while let Some(_result) = running_operations.next().await {
        continue;
    }
    io.send_message(DocToHubMsgPayload::Terminated);
    state.lock().unwrap().set_run_state(RunState::Stopped);
    tracing::debug!("document actor runtime loop exited cleanly");
}

fn generate_sync_messages(
    io: ActorIoAccess,
    state: Arc<Mutex<ActorState>>,
    now: Arc<Mutex<UnixTimestamp>>,
) {
    // Something happened, make sure we send any sync messages we need to send
    let doc_id = state.lock().unwrap().document_id.clone();
    for (conn_id, msgs) in state
        .lock()
        .unwrap()
        .generate_sync_messages(*now.lock().unwrap())
    {
        for msg in msgs {
            io.send_message(DocToHubMsgPayload::SendSyncMessage {
                connection_id: conn_id,
                document_id: doc_id.clone(),
                message: msg,
            });
        }
    }
}

fn save_new_changes(
    io: ActorIoAccess,
    state: Arc<Mutex<ActorState>>,
    running_operations: &mut FuturesUnordered<BoxFuture<'static, Result<(), DocumentError>>>,
) {
    // Make sure we save any new changes
    let new_jobs = state.lock().unwrap().pop_new_jobs();
    if !new_jobs.is_empty() {
        for job in new_jobs {
            let io = io.clone();
            let state = state.clone();
            running_operations.push(
                async move {
                    let result = match job {
                        Job::Put { key, data } => {
                            io.put(key.clone(), data).await;
                            JobComplete::Put(key)
                        }
                        Job::Delete(storage_key) => {
                            io.delete(storage_key.clone()).await;
                            JobComplete::Delete(storage_key)
                        }
                    };
                    state.lock().unwrap().mark_job_complete(result);
                    Ok(())
                }
                .boxed(),
            );
        }
    }
}

fn enqueue_announce_policy_checks(
    io: ActorIoAccess,
    state: Arc<Mutex<ActorState>>,
    running_operations: &mut FuturesUnordered<BoxFuture<'static, Result<(), DocumentError>>>,
) {
    let check_policy_tasks = state.lock().unwrap().pop_announce_policy_tasks();
    for (peer_id, conn_id) in check_policy_tasks {
        tracing::trace!(?peer_id, ?conn_id, "checking announce policy");
        running_operations.push({
            let state = state.clone();
            let io = io.clone();
            async move {
                let should_announce = io.check_announce_policy(peer_id.clone()).await;
                tracing::trace!(
                    ?peer_id,
                    ?conn_id,
                    ?should_announce,
                    "announce policy check result",
                );
                if should_announce {
                    state
                        .lock()
                        .unwrap()
                        .set_announce_policy(io, conn_id, AnnouncePolicy::Announce)
                } else {
                    state.lock().unwrap().set_announce_policy(
                        io,
                        conn_id,
                        AnnouncePolicy::DontAnnounce,
                    )
                }
                Ok::<_, DocumentError>(())
            }
            .boxed()
        })
    }
}

/// Handle an input message by spawning the appropriate async operation.
///
/// This function maps input messages to their corresponding async operation
/// handlers, providing a clean separation between message routing and
/// operation implementation.
async fn handle_input(
    now: Arc<Mutex<UnixTimestamp>>,
    state: Arc<Mutex<ActorState>>,
    io: ActorIoAccess,
    input: ActorInput,
) -> Result<(), DocumentError> {
    match input {
        ActorInput::Terminate => {
            // Exit the runtime loop after termination
            std::future::pending::<()>().await;
            unreachable!()
        }
        ActorInput::HandleDocMessage {
            connection_id,
            message,
        } => {
            state.lock().unwrap().handle_doc_message(
                *now.lock().unwrap(),
                io.clone(),
                connection_id,
                message,
            );
        }
        ActorInput::NewConnection {
            connection_id,
            peer_id,
        } => {
            state.lock().unwrap().add_connection(connection_id, peer_id);
        }
        ActorInput::ConnectionClosed { connection_id } => {
            state.lock().unwrap().remove_connection(connection_id);
        }
        ActorInput::Request => {
            load(io.clone(), state, now).await?;
        }
        ActorInput::Tick => {}
    }
    Ok(())
}

async fn load(
    io: ActorIoAccess,
    state: Arc<Mutex<ActorState>>,
    now: Arc<Mutex<UnixTimestamp>>,
) -> Result<(), DocumentError> {
    state.lock().unwrap().ensure_request(io.clone());
    tracing::debug!("loading document from storage");
    let io = io.clone();
    let doc_id = state.lock().unwrap().document_id.clone();

    let snapshot_prefix = StorageKey::from(vec![doc_id.to_string(), "snapshot".to_string()]);
    let snapshots = io.load_range(snapshot_prefix).await;

    let incremental_prefix = StorageKey::from(vec![doc_id.to_string(), "incremental".to_string()]);
    let incrementals = io.load_range(incremental_prefix).await;

    state
        .lock()
        .unwrap()
        .handle_load(*now.lock().unwrap(), io.clone(), snapshots, incrementals);
    Ok(())
}
