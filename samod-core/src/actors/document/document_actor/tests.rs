mod update_share_policy;

use super::*;
use crate::{ShareAccess, ShareConfig, actors::messages::HubToDocMsgPayload};
use automerge::{
    ROOT, ReadDoc,
    sync::{self, SyncDoc},
    transaction::Transactable,
};

fn incoming_changes(v2: bool) -> SyncMessage {
    let mut sender = Automerge::new();
    sender
        .transact::<_, _, automerge::AutomergeError>(|tx| tx.put(ROOT, "remote", true))
        .unwrap();
    let mut state = sync::State::new();
    let hello = Automerge::new()
        .generate_sync_message(&mut sync::State::new())
        .unwrap();
    sender.receive_sync_message(&mut state, hello).unwrap();
    if !v2 {
        state.their_capabilities = None;
    }
    let message = sender.generate_sync_message(&mut state).unwrap();
    assert!(!message.changes.is_empty());
    SyncMessage::Sync {
        data: message.encode(),
    }
}

fn new_actor(
    ready: bool,
    initial: Option<SyncMessage>,
) -> (DocumentActor, DocActorResult, ConnectionId, IoTaskId) {
    let conn = ConnectionId::new();
    let args = SpawnArgs {
        local_peer_id: "local".into(),
        actor_id: DocumentActorId::new(),
        document_id: DocumentId::new(&mut rand::rng()),
        initial_content: ready.then(Automerge::new),
        initial_connections: HashMap::from([(
            conn,
            ("remote".into(), initial.map(DocMessage::Sync)),
        )]),
    };
    let (actor, result) = DocumentActor::new(UnixTimestamp::now(), args);
    let policy_task = result
        .io_tasks
        .iter()
        .find(|task| matches!(task.action, DocumentIoTask::CheckSharePolicy { .. }))
        .unwrap()
        .task_id;
    (actor, result, conn, policy_task)
}

fn complete_policy(
    actor: &mut DocumentActor,
    task_id: IoTaskId,
    config: ShareConfig,
) -> DocActorResult {
    actor
        .handle_io_complete(
            UnixTimestamp::now(),
            IoResult {
                task_id,
                payload: DocumentIoResult::CheckSharePolicy(config),
            },
        )
        .unwrap()
}

fn has_network_output(result: &DocActorResult) -> bool {
    result.outgoing_messages.iter().any(|msg| match &msg.0 {
        DocToHubMsgPayload::SendSyncMessage { .. } => true,
        DocToHubMsgPayload::Broadcast { connections, .. } => !connections.is_empty(),
        _ => false,
    })
}

#[test]
fn initial_messages_wait_for_policy_in_ready_and_loading_phases() {
    for ready in [false, true] {
        for policy_before_load in [false, true] {
            for v2 in [false, true] {
                for config in [
                    ShareConfig::none(),
                    ShareConfig::read_only(),
                    ShareConfig::read_write(),
                ] {
                    let (mut actor, result, conn, task_id) =
                        new_actor(ready, Some(incoming_changes(v2)));
                    assert!(!has_network_output(&result));
                    assert!(result.change_events.is_empty());
                    assert!(actor.document().get_heads().is_empty());
                    assert_eq!(result.pending_sync_messages, 1);
                    assert!(!has_network_output(
                        &actor.broadcast(UnixTimestamp::now(), vec![1])
                    ));

                    let mut out = DocActorResult::new();
                    if !ready && !policy_before_load {
                        actor.handle_load(
                            UnixTimestamp::now(),
                            &mut out,
                            &HashMap::new(),
                            &HashMap::new(),
                        );
                        assert!(actor.document().get_heads().is_empty());
                        assert!(!has_network_output(&out));
                    }
                    let decision = complete_policy(&mut actor, task_id, config);
                    if !ready && policy_before_load {
                        assert!(actor.document().get_heads().is_empty());
                        actor.handle_load(
                            UnixTimestamp::now(),
                            &mut out,
                            &HashMap::new(),
                            &HashMap::new(),
                        );
                        actor.step(UnixTimestamp::now(), &mut out);
                    }
                    assert_eq!(
                        actor.document().get(ROOT, "remote").unwrap().is_some(),
                        config.access == ShareAccess::ReadWrite
                    );
                    assert_eq!(actor.peers()[&conn].share_config, Some(config));
                    if config.access != ShareAccess::ReadWrite {
                        assert!(decision.change_events.is_empty());
                        assert!(out.change_events.is_empty());
                        assert!(actor.document().get_heads().is_empty());
                    }
                }
            }
        }
    }
}

#[test]
fn unsolicited_changes_are_ignored_even_if_sender_ignores_read_only_flag() {
    for v2 in [false, true] {
        for request in [false, true] {
            let (mut actor, _, conn, task_id) = new_actor(true, None);
            complete_policy(&mut actor, task_id, ShareConfig::read_only());
            let SyncMessage::Sync { data } = incoming_changes(v2) else {
                unreachable!()
            };
            let message = if request {
                SyncMessage::Request { data }
            } else {
                SyncMessage::Sync { data }
            };
            let result = actor
                .handle_message(
                    UnixTimestamp::now(),
                    HubToDocMsg(HubToDocMsgPayload::HandleDocMessage {
                        connection_id: conn,
                        message: DocMessage::Sync(message),
                        received_at: UnixTimestamp::now(),
                    }),
                )
                .unwrap();
            assert!(actor.document().get_heads().is_empty());
            assert!(result.change_events.is_empty());
            assert!(!result.io_tasks.iter().any(|task| matches!(
                task.action,
                DocumentIoTask::Storage(crate::io::StorageTask::Put { .. })
            )));
        }
    }
}

#[test]
fn closed_connection_discards_pending_messages_and_late_policy_results() {
    let (mut actor, _, conn, task_id) = new_actor(true, Some(incoming_changes(true)));
    actor
        .handle_message(
            UnixTimestamp::now(),
            HubToDocMsg(HubToDocMsgPayload::ConnectionClosed {
                connection_id: conn,
            }),
        )
        .unwrap();
    let result = complete_policy(&mut actor, task_id, ShareConfig::read_write());
    assert!(actor.document().get_heads().is_empty());
    assert!(!has_network_output(&result));
    assert_eq!(result.pending_sync_messages, 0);
    assert!(actor.peers().is_empty());
}

#[test]
fn overflowing_pending_policy_queue_disconnects_without_applying_changes() {
    let (mut actor, _, conn, task_id) = new_actor(true, None);
    let mut disconnected = false;
    for _ in 0..65 {
        let result = actor
            .handle_message(
                UnixTimestamp::now(),
                HubToDocMsg(HubToDocMsgPayload::HandleDocMessage {
                    connection_id: conn,
                    message: DocMessage::Sync(SyncMessage::DocUnavailable),
                    received_at: UnixTimestamp::now(),
                }),
            )
            .unwrap();
        disconnected |= result.outgoing_messages.iter().any(|msg| {
            matches!(
                msg.0, DocToHubMsgPayload::Disconnect { connection_id, .. } if connection_id == conn
            )
        });
    }
    assert!(disconnected);
    assert!(actor.peers().is_empty());
    let result = complete_policy(&mut actor, task_id, ShareConfig::read_write());
    assert_eq!(result.pending_sync_messages, 0);
    assert!(actor.document().get_heads().is_empty());
}

#[test]
fn pending_policy_queue_limits_bytes_but_allows_one_large_document() {
    let (mut actor, _, conn, _) = new_actor(true, None);
    let conn = actor.peer_connections.get_mut(&conn).unwrap();
    let now = UnixTimestamp::now();
    assert!(conn.buffer_sync_message(
        SyncMessage::Sync {
            data: vec![0; 10 * 1024 * 1024]
        },
        now
    ));
    assert!(!conn.buffer_sync_message(
        SyncMessage::Sync {
            data: vec![0; 10 * 1024 * 1024]
        },
        now
    ));
    conn.pending_sync_messages.clear();
    assert!(conn.buffer_sync_message(
        SyncMessage::Sync {
            data: vec![0; 17 * 1024 * 1024]
        },
        now
    ));
    assert!(!conn.buffer_sync_message(SyncMessage::Sync { data: vec![0] }, now));
}

#[test]
fn resetting_sync_state_preserves_read_only() {
    let (mut actor, _, conn, task_id) = new_actor(true, None);
    complete_policy(&mut actor, task_id, ShareConfig::read_only());
    let conn = actor.peer_connections.get_mut(&conn).unwrap();
    conn.reset_sync_state();
    assert!(conn.sync_state.read_only);
}
