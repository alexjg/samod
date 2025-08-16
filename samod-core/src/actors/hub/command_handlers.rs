use std::collections::HashMap;

use crate::{
    ConnectionId, DocumentActorId, DocumentId, PeerId,
    actors::{
        document::DocumentStatus,
        hub::connection::{Connection, ConnectionArgs, ReceiveEvent},
        messages::{DocMessage, HubToDocMsgPayload},
    },
    ephemera::EphemeralMessage,
    network::{ConnDirection, ConnectionEvent, PeerInfo, wire_protocol::WireMessage},
};
use automerge::Automerge;

use super::{Command, CommandId, CommandResult, task_context::TaskContext};

/// Handle a command, returning `Some(CommandResult)` if the command was handled
/// immediately and `None` if it will be completed asynchronously
pub(crate) fn handle_command<R: rand::Rng + Send + Clone>(
    ctx: TaskContext<R>,
    command_id: CommandId,
    command: Command,
) -> Option<CommandResult> {
    match command {
        Command::CreateConnection { direction } => Some(handle_create_connection(ctx, direction)),
        Command::Receive { connection_id, msg } => Some(handle_receive(ctx, connection_id, msg)),
        Command::ActorReady { document_id: _ } => Some(CommandResult::ActorReady),
        Command::CreateDocument { content } => {
            handle_create_document(ctx, command_id, *content);
            None
        }
        Command::FindDocument { document_id } => handle_find_document(ctx, command_id, document_id),
    }
}

fn handle_create_connection<R: rand::Rng + Send + Clone>(
    ctx: TaskContext<R>,
    direction: ConnDirection,
) -> CommandResult {
    let local_peer_id = ctx.state().peer_id();
    let local_metadata = ctx.state().get_local_metadata();

    let connection = Connection::new_handshaking(
        ctx.io(),
        ConnectionArgs {
            direction,
            local_peer_id: local_peer_id.clone(),
            local_metadata: Some(local_metadata.clone()),
            created_at: ctx.now(),
        },
    );

    let connection_id = connection.id();

    tracing::debug!(?connection_id, ?direction, "creating new connection");

    ctx.state().add_connection(connection_id, connection);

    CommandResult::CreateConnection { connection_id }
}

fn handle_receive<R: rand::Rng + Send + Clone>(
    ctx: TaskContext<R>,
    connection_id: ConnectionId,
    msg: Vec<u8>,
) -> CommandResult {
    tracing::trace!(?connection_id, msg_bytes = msg.len(), "receive");
    let Some(conn) = ctx.state().get_connection(&connection_id) else {
        tracing::warn!(?connection_id, "receive command for nonexistent connection");

        return CommandResult::Receive {
            connection_id,
            error: Some("Connection not found".to_string()),
        };
    };

    let msg = match WireMessage::decode(&msg) {
        Ok(msg) => msg,
        Err(e) => {
            tracing::warn!(
                ?connection_id,
                err=?e,
                "failed to decode message: {}",
                e
            );
            let error_msg = format!("Message decode error: {e}");
            ctx.fail_connection_with_disconnect(connection_id, error_msg);

            return CommandResult::Receive {
                connection_id,
                error: Some(format!("Decode error: {e}")),
            };
        }
    };

    for evt in conn.receive(msg) {
        match evt {
            ReceiveEvent::HandshakeComplete { remote_peer_id } => {
                tracing::debug!(?connection_id, ?remote_peer_id, "handshake completed");
                // Emit handshake completed event
                let peer_info = PeerInfo {
                    peer_id: remote_peer_id.clone(),
                    metadata: Some(ctx.state().get_local_metadata()),
                    protocol_version: "1".to_string(),
                };
                ctx.emit_connection_event(ConnectionEvent::HandshakeCompleted {
                    connection_id,
                    peer_info: peer_info.clone(),
                });
            }
            ReceiveEvent::SyncMessage {
                doc_id,
                sender_id: _,
                target_id,
                msg,
            } => handle_doc_message(
                ctx.clone(),
                connection_id,
                target_id,
                doc_id,
                DocMessage::Sync(msg),
            ),
            ReceiveEvent::EphemeralMessage {
                doc_id,
                sender_id,
                target_id,
                count,
                session_id,
                msg,
            } => {
                let msg = EphemeralMessage {
                    sender_id,
                    session_id,
                    count,
                    data: msg,
                };
                if let Some(msg) = ctx.state().receive_ephemeral_message(msg) {
                    handle_doc_message(
                        ctx.clone(),
                        connection_id,
                        target_id,
                        doc_id,
                        DocMessage::Ephemeral(msg),
                    )
                }
            }
        }
    }
    CommandResult::Receive {
        connection_id,
        error: None,
    }
}

fn handle_doc_message<R: rand::Rng + Send + Clone>(
    ctx: TaskContext<R>,
    connection_id: ConnectionId,
    target_id: PeerId,
    doc_id: DocumentId,
    msg: DocMessage,
) {
    // Validate this request is for us
    if target_id != ctx.state().peer_id() {
        tracing::trace!(?connection_id, ?msg, "ignoring message for another peer");
    }

    // Ensure there's a document actor for this document
    if let Some(existing_actor) = ctx.state().find_actor_for_document(&doc_id) {
        // Forward the request to the document actor
        ctx.send_to_actor(
            existing_actor.actor_id,
            HubToDocMsgPayload::HandleDocMessage {
                connection_id,
                message: msg,
            },
        );
    } else {
        spawn_actor(ctx.clone(), doc_id, None, Some((connection_id, msg)));
    }
}

#[tracing::instrument(skip(ctx, init_doc), fields(command_id = %command_id))]
fn handle_create_document<R: rand::Rng + Send + Clone>(
    mut ctx: TaskContext<R>,
    command_id: CommandId,
    init_doc: Automerge,
) {
    // Generate new document ID
    let document_id = DocumentId::new(ctx.rng());

    tracing::debug!(%document_id, "creating new document");

    let actor_id = spawn_actor(ctx.clone(), document_id, Some(init_doc), None);

    // Queue command for completion when actor reports ready
    ctx.state().add_pending_create_command(actor_id, command_id);
}

#[tracing::instrument(skip(ctx), fields(document_id = %document_id))]
fn handle_find_document<R: rand::Rng + Send + Clone>(
    ctx: TaskContext<R>,
    command_id: CommandId,
    document_id: DocumentId,
) -> Option<CommandResult> {
    tracing::debug!("find document command received");
    // Check if actor already exists and is ready
    if let Some(actor_info) = ctx.state().find_actor_for_document(&document_id) {
        tracing::trace!(%actor_info.actor_id, ?actor_info.status, "found existing actor for document");
        return match actor_info.status {
            DocumentStatus::Spawned | DocumentStatus::Requesting | DocumentStatus::Loading => {
                ctx.state()
                    .add_pending_find_command(document_id, command_id);
                None
            }
            DocumentStatus::Ready => {
                // Document is ready
                Some(CommandResult::FindDocument {
                    found: true,
                    actor_id: actor_info.actor_id,
                })
            }
            DocumentStatus::NotFound => {
                // In this case we need to restart the request process
                tracing::trace!(%actor_info.actor_id, ?actor_info.status, "re-requesting document from actor");
                ctx.send_to_actor(actor_info.actor_id, HubToDocMsgPayload::RequestAgain);

                ctx.state()
                    .add_pending_find_command(document_id, command_id);
                None
            }
        };
    }

    tracing::trace!("no existing actor found for document, spawning new actor");

    spawn_actor(ctx.clone(), document_id.clone(), None, None);

    ctx.state()
        .add_pending_find_command(document_id, command_id);
    None
}

fn spawn_actor<R: rand::Rng + Send + Clone>(
    ctx: TaskContext<R>,
    document_id: DocumentId,
    initial_doc: Option<Automerge>,
    from_sync_msg: Option<(ConnectionId, DocMessage)>,
) -> DocumentActorId {
    // Create new actor to find/load the document
    let actor_id = DocumentActorId::new();

    // Create the actor and initialize it
    ctx.state()
        .add_document_actor(actor_id, document_id.clone());

    let mut initial_connections: HashMap<ConnectionId, (PeerId, Option<DocMessage>)> = ctx
        .state()
        .established_peers()
        .iter()
        .map(|(c, p)| (*c, (p.clone(), None)))
        .collect();

    for conn in initial_connections.keys() {
        ctx.state()
            .add_document_to_connection(conn, document_id.clone());
    }

    if let Some((conn_id, msg)) = from_sync_msg {
        if let Some((_, sync_msg)) = initial_connections.get_mut(&conn_id) {
            *sync_msg = Some(msg);
        }
    }

    ctx.spawn_actor(actor_id, document_id, initial_doc, initial_connections);

    actor_id
}
