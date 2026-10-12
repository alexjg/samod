use super::*;

fn update_policy(actor: &mut DocumentActor, config: ShareConfig) -> DocActorResult {
    let result = actor
        .handle_message(
            UnixTimestamp::now(),
            HubToDocMsg::update_share_policy("remote".into(), config),
        )
        .unwrap();
    assert!(
        policy_tasks(&result).is_empty(),
        "updates must not recheck policy"
    );
    result
}

fn policy_tasks(result: &DocActorResult) -> Vec<IoTaskId> {
    result
        .io_tasks
        .iter()
        .filter_map(|task| {
            matches!(task.action, DocumentIoTask::CheckSharePolicy { .. }).then_some(task.task_id)
        })
        .collect()
}

#[test]
fn regrant_after_dropping_a_modern_peers_first_reply_converges() {
    let messages = |result: DocActorResult| {
        result
            .outgoing_messages
            .into_iter()
            .filter_map(|msg| match msg.0 {
                DocToHubMsgPayload::SendSyncMessage { message, .. } => Some(message),
                _ => None,
            })
            .collect::<Vec<_>>()
    };
    let receive = |actor: &mut DocumentActor, conn, msg: sync::Message| {
        actor
            .handle_message(
                UnixTimestamp::now(),
                HubToDocMsg(HubToDocMsgPayload::HandleDocMessage {
                    connection_id: conn,
                    message: DocMessage::Sync(SyncMessage::Sync { data: msg.encode() }),
                    received_at: UnixTimestamp::now(),
                }),
            )
            .unwrap()
    };
    for grant in [ShareConfig::read_only(), ShareConfig::read_write()] {
        let (mut actor, _, conn, initial) = new_actor(true, None);
        actor
            .with_document(UnixTimestamp::now(), |doc| {
                doc.transact::<_, _, automerge::AutomergeError>(|tx| tx.put(ROOT, "local", true))
                    .unwrap();
            })
            .unwrap();
        let initial = complete_policy(&mut actor, initial, ShareConfig::read_only());
        let mut initial = messages(initial);
        assert_eq!(initial.len(), 1);
        let SyncMessage::Sync { data } = initial.remove(0) else {
            panic!("expected initial sync message");
        };
        let mut remote = Automerge::new();
        let mut remote_state = sync::State::new();
        remote
            .receive_sync_message(&mut remote_state, sync::Message::decode(&data).unwrap())
            .unwrap();
        let reply = remote.generate_sync_message(&mut remote_state).unwrap();
        assert!(
            reply
                .flags
                .unwrap()
                .contains(sync::MessageFlags::SUPPORTS_SYNC_RESET)
        );

        // Deny before the first reply arrives. It must be rejected rather than
        // delivered to Automerge, leaving the modern peer's capabilities unknown.
        let denied = update_policy(&mut actor, ShareConfig::none());
        assert!(messages(denied).is_empty());
        let rejected = messages(receive(&mut actor, conn, reply));
        assert!(matches!(rejected.as_slice(), [SyncMessage::DocUnavailable]));
        assert!(actor.peers()[&conn].last_received.is_none());

        let result = update_policy(&mut actor, grant);
        assert_eq!(result.sync_message_stats.len(), 2, "one timing per message");
        let mut pending: std::collections::VecDeque<_> = messages(result).into();
        assert_eq!(pending.len(), 2);
        let mut rounds = 0;
        while let Some(message) = pending.pop_front() {
            rounds += 1;
            assert!(rounds < 20, "sync must become quiescent");
            let SyncMessage::Sync { data } = message else {
                panic!("expected sync message");
            };
            let message = sync::Message::decode(&data).unwrap();
            if rounds == 1 {
                assert!(message.heads.is_empty());
            } else if rounds == 2 {
                assert_eq!(message.heads, actor.document().get_heads());
            }
            remote
                .receive_sync_message(&mut remote_state, message)
                .unwrap();
            let reply = remote.generate_sync_message(&mut remote_state);
            if rounds == 1 {
                assert!(
                    reply.is_none(),
                    "the empty peer does not acknowledge the reset"
                );
            }
            if let Some(reply) = reply {
                pending.extend(messages(receive(&mut actor, conn, reply)));
            }
        }
        assert_eq!(remote.get_heads(), actor.document().get_heads());
        assert!(remote.get(ROOT, "local").unwrap().is_some());
        assert!(remote.generate_sync_message(&mut remote_state).is_none());
        let mut idle = DocActorResult::new();
        actor.generate_sync_messages(UnixTimestamp::now(), &mut idle);
        assert!(messages(idle).is_empty());
    }
}

#[test]
fn updates_supersede_initial_checks_in_either_completion_order() {
    for initial_first in [true, false] {
        let (mut actor, _, conn, initial) = new_actor(true, Some(incoming_changes(true)));
        if initial_first {
            complete_policy(&mut actor, initial, ShareConfig::read_only());
        }
        update_policy(&mut actor, ShareConfig::none());
        update_policy(&mut actor, ShareConfig::read_only());
        if !initial_first {
            let result = complete_policy(&mut actor, initial, ShareConfig::read_write());
            assert!(!has_network_output(&result));
        }
        assert_eq!(
            actor.peers()[&conn].share_config,
            Some(ShareConfig::read_only())
        );
        assert!(actor.document().get_heads().is_empty());
        assert!(actor.check_policy_tasks.is_empty());
    }
}

#[test]
fn identical_and_announcement_only_updates_preserve_sync_state() {
    for announce in [true, false] {
        let (mut actor, _, conn, task) = new_actor(true, None);
        complete_policy(&mut actor, task, ShareConfig::read_write());
        let before = actor.peer_connections[&conn].sync_state.clone();
        let config = ShareConfig::read_write().with_announce(announce);
        update_policy(&mut actor, config);
        assert_eq!(actor.peers()[&conn].share_config, Some(config));
        assert_eq!(actor.peer_connections[&conn].sync_state, before);
    }
}

#[test]
fn ephemera_use_updated_permissions_immediately() {
    let (mut actor, _, conn, initial) = new_actor(true, None);
    complete_policy(&mut actor, initial, ShareConfig::read_write());
    for config in [ShareConfig::none(), ShareConfig::read_only()] {
        update_policy(&mut actor, config);
        let result = actor
            .handle_message(
                UnixTimestamp::now(),
                HubToDocMsg(HubToDocMsgPayload::HandleDocMessage {
                    connection_id: conn,
                    message: DocMessage::Ephemeral(crate::ephemera::EphemeralMessage {
                        sender_id: "remote".into(),
                        session_id: "session".to_string().into(),
                        count: 1,
                        data: vec![1],
                    }),
                    received_at: UnixTimestamp::now(),
                }),
            )
            .unwrap();
        assert_eq!(!result.ephemeral_messages.is_empty(), config.can_read());
    }
}

#[test]
fn updates_drain_buffered_messages_under_the_new_permissions() {
    for config in [
        ShareConfig::none(),
        ShareConfig::read_only(),
        ShareConfig::read_write(),
    ] {
        let (mut actor, _, conn, initial) = new_actor(true, Some(incoming_changes(true)));
        assert_eq!(actor.peer_connections[&conn].pending_sync_messages.len(), 1);
        update_policy(&mut actor, config);
        assert!(
            actor.peer_connections[&conn]
                .pending_sync_messages
                .is_empty()
        );
        assert_eq!(!actor.document().get_heads().is_empty(), config.can_write());
        complete_policy(&mut actor, initial, ShareConfig::read_write());
        assert_eq!(actor.peers()[&conn].share_config, Some(config));
        assert_eq!(!actor.document().get_heads().is_empty(), config.can_write());
    }
}

#[test]
fn updates_apply_to_all_matching_connections_including_pending_checks() {
    let (mut actor, _, first, initial) = new_actor(true, None);
    complete_policy(&mut actor, initial, ShareConfig::read_write());
    let second = ConnectionId::new();
    let other = ConnectionId::new();
    let mut tasks = Vec::new();
    for (connection_id, peer_id) in [(second, "remote"), (other, "other")] {
        let result = actor
            .handle_message(
                UnixTimestamp::now(),
                HubToDocMsg(HubToDocMsgPayload::NewConnection {
                    connection_id,
                    peer_id: peer_id.into(),
                }),
            )
            .unwrap();
        tasks.push(policy_tasks(&result)[0]);
    }
    for config in [
        ShareConfig::none(),
        ShareConfig::read_only(),
        ShareConfig::read_write(),
    ] {
        update_policy(&mut actor, config);
        for conn in [first, second] {
            assert_eq!(actor.peers()[&conn].share_config, Some(config));
        }
        assert_eq!(actor.peers()[&other].share_config, None);
    }
    complete_policy(&mut actor, tasks[0], ShareConfig::none());
    assert_eq!(
        actor.peers()[&second].share_config,
        Some(ShareConfig::read_write())
    );
    complete_policy(&mut actor, tasks[1], ShareConfig::read_only());
    assert_eq!(
        actor.peers()[&other].share_config,
        Some(ShareConfig::read_only())
    );
}

#[test]
fn update_without_connections_is_a_noop_and_reconnection_still_checks_policy() {
    let (mut actor, _, conn, initial) = new_actor(true, None);
    actor
        .handle_message(
            UnixTimestamp::now(),
            HubToDocMsg(HubToDocMsgPayload::ConnectionClosed {
                connection_id: conn,
            }),
        )
        .unwrap();
    let result = update_policy(&mut actor, ShareConfig::read_write());
    assert!(!has_network_output(&result));
    let new_conn = ConnectionId::new();
    let result = actor
        .handle_message(
            UnixTimestamp::now(),
            HubToDocMsg(HubToDocMsgPayload::NewConnection {
                connection_id: new_conn,
                peer_id: "remote".into(),
            }),
        )
        .unwrap();
    assert_eq!(policy_tasks(&result).len(), 1);
    complete_policy(&mut actor, initial, ShareConfig::read_write());
    assert_eq!(actor.peers()[&new_conn].share_config, None);
    complete_policy(&mut actor, policy_tasks(&result)[0], ShareConfig::none());
    assert_eq!(
        actor.peers()[&new_conn].share_config,
        Some(ShareConfig::none())
    );
}

#[test]
fn update_during_loading_survives_both_completion_orders() {
    for policy_before_load in [false, true] {
        let (mut actor, _, conn, initial) = new_actor(false, Some(incoming_changes(true)));
        let mut out = DocActorResult::new();
        if !policy_before_load {
            actor.handle_load(
                UnixTimestamp::now(),
                &mut out,
                &HashMap::new(),
                &HashMap::new(),
            );
            assert!(actor.document().get_heads().is_empty());
        }
        update_policy(&mut actor, ShareConfig::read_only());
        complete_policy(&mut actor, initial, ShareConfig::read_write());
        if policy_before_load {
            actor.handle_load(
                UnixTimestamp::now(),
                &mut out,
                &HashMap::new(),
                &HashMap::new(),
            );
        }
        assert!(actor.document().get_heads().is_empty());
        assert_eq!(
            actor.peers()[&conn].share_config,
            Some(ShareConfig::read_only())
        );
    }
}
