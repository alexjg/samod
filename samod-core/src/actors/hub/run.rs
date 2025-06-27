use std::{cell::RefCell, rc::Rc};

use futures::{StreamExt, channel::mpsc, stream::FuturesUnordered};

use crate::{
    UnixTimestamp,
    actors::{
        RunState,
        driver::ActorIo,
        hub::{command_handlers, task_context::TaskContext},
        messages::{DocToHubMsgPayload, HubToDocMsgPayload},
    },
    network::{ConnectionEvent, wire_protocol::WireMessageBuilder},
};

use super::{Hub, State, io::HubIoAction};

mod hub_input;
pub(crate) use hub_input::HubInput;
mod hub_output;
pub(crate) use hub_output::HubOutput;

pub(crate) async fn run<R: rand::Rng + Clone + 'static>(
    rng: R,
    now: Rc<RefCell<UnixTimestamp>>,
    state: Rc<RefCell<State>>,
    mut rx_input: mpsc::UnboundedReceiver<HubInput>,
    io: ActorIo<Hub>,
) {
    // Collection of futures representing commands that are currently executing
    let mut running_commands = FuturesUnordered::new();

    let ctx = TaskContext::new(rng, now.clone(), io.clone(), state.clone());

    loop {
        futures::select! {
            input = rx_input.select_next_some() => {
                match input {
                    HubInput::Stop => {
                        // Stop the hub event loop
                        tracing::info!("Stopping hub event loop");
                        break;
                    },
                    HubInput::Command{command_id, command} => {
                        let ctx = ctx.clone();
                        let handler = async move {
                            let result = command_handlers::handle_command(
                                ctx,
                                command_id,
                                *command,
                            ).await;
                            (command_id, result)
                        };
                        running_commands.push(handler);
                    },
                    HubInput::Tick => {
                        // Tick events are used to trigger periodic processing
                        // but don't spawn new command futures
                    },
                    HubInput::ActorMessage { actor_id, message } => {
                        match message {
                            DocToHubMsgPayload::DocumentStatusChanged {new_status} => {
                                ctx.state().update_document_status(actor_id, new_status);
                            }
                            DocToHubMsgPayload::SendSyncMessage {
                                document_id, connection_id, message
                            } => {
                                if let Some(target_id) = ctx.state().remote_peer_id(connection_id) {
                                    let wire_message = WireMessageBuilder{
                                        sender_id: ctx.state().peer_id().clone(),
                                        target_id,
                                        document_id,
                                    }.from_sync_message(message);
                                    let task = HubIoAction::Send{connection_id, msg: wire_message.encode()};
                                    io.fire_and_forget_io(task);
                                } else {
                                    tracing::warn!(?connection_id, "received SendSyncMessage for unknown connection ID");
                                }
                            }
                            DocToHubMsgPayload::PeerStatesChanged { new_states } => {
                                ctx.state().update_peer_states(actor_id, new_states);
                            }
                            DocToHubMsgPayload::Broadcast { connections, msg } => {
                                ctx.broadcast(actor_id, connections, msg);
                            },
                            DocToHubMsgPayload::Terminated => {
                                // An actor has terminated, remove it from the state
                                panic!("unexpected document actor termination");
                            },
                        }
                    },
                    HubInput::ConnectionLost { connection_id } => {
                        if let Some(_connection) = ctx.state().remove_connection(&connection_id) {
                            ctx.emit_disconnect_event(connection_id, "Connection lost externally".to_string());
                            ctx.notify_doc_actors_of_removed_connection(connection_id);
                        }
                    },
                }
            }
            finished_command = running_commands.select_next_some() => {
                let (command_id, result) = finished_command;
                io.emit_event(HubOutput::CommandCompleted{ command_id, result });
            }
        }

        // Notify document actors of any closed connections
        for conn_id in ctx.state().pop_closed_connections() {
            for doc in ctx.state().document_actors() {
                io.emit_event(HubOutput::SendToActor {
                    actor_id: doc.actor_id,
                    message: HubToDocMsgPayload::ConnectionClosed {
                        connection_id: conn_id,
                    },
                });
            }
        }

        // Now check that every connection is connected to ever document
        for (actor_id, conn_id, peer_id) in ctx.state().ensure_connections() {
            io.emit_event(HubOutput::SendToActor {
                actor_id,
                message: HubToDocMsgPayload::NewConnection {
                    connection_id: conn_id,
                    peer_id,
                },
            });
        }

        // Notify any listeners of updated connection info ("info" here is for monitoring purposes,
        // things like the last time we sent a message and the heads of each document according
        // to the connection and so on).
        for (conn_id, new_state) in ctx.state().pop_new_connection_info() {
            io.emit_event(HubOutput::ConnectionEvent {
                event: ConnectionEvent::StateChanged {
                    connection_id: conn_id,
                    new_state,
                },
            });
        }
    }

    // We're stopping the hub
    tracing::info!("stopping hub, terminating all document actors");

    // Stop all the document actors
    for doc in ctx.state().document_actors() {
        io.emit_event(HubOutput::SendToActor {
            actor_id: doc.actor_id,
            message: HubToDocMsgPayload::Terminate,
        });
    }

    // Now wait for all the actors to finish, keep running any commands that are
    // still executing until all the actors have stopped
    loop {
        if ctx.state().document_actors().is_empty() {
            break;
        }
        futures::select! {
            input = rx_input.select_next_some() => {
                match input {
                    HubInput::Command { command_id, command: _ } => {
                        // Ignore new commands
                        tracing::warn!(?command_id, "received new command while stopping hub, ignoring");
                    }
                    HubInput::Tick => continue,
                    HubInput::ActorMessage { actor_id, message } => match message {
                        DocToHubMsgPayload::Terminated => {
                            // An actor has terminated, remove it from the state
                            ctx.state().remove_document_actor(actor_id);
                        },
                        _ => continue,
                    }
                    HubInput::ConnectionLost { connection_id } => {
                        if let Some(_connection) = ctx.state().remove_connection(&connection_id) {
                            ctx.emit_disconnect_event(connection_id, "Connection lost externally".to_string());
                            ctx.notify_doc_actors_of_removed_connection(connection_id);
                        }
                    },
                    HubInput::Stop => continue,
                }
            }
            finished_command = running_commands.select_next_some() => {
                let (command_id, result) = finished_command;
                io.emit_event(HubOutput::CommandCompleted{ command_id, result });
            }
        }
    }

    state.borrow_mut().set_run_state(RunState::Stopped);
}
