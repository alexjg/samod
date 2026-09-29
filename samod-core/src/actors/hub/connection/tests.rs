use std::{collections::HashMap, time::Duration};

use rand::{SeedableRng, rngs::StdRng};

use crate::{
    CommandResult, ConnectionId, DocumentId, LoaderState, PeerId, SamodLoader, UnixTimestamp,
    actors::{
        hub::{Hub, HubEvent, HubResults, io::HubIoAction},
        messages::HubToDocMsgPayload,
    },
    io::{IoResult, StorageResult, StorageTask},
    network::{
        BackoffConfig, ConnDirection, ConnectionEvent, ConnectionOwner, DialerConfig,
        ListenerConfig, wire_protocol::WireMessage,
    },
};

struct Fixture {
    hub: Hub,
    rng: StdRng,
    now: UnixTimestamp,
    connection: ConnectionId,
    owner: ConnectionOwner,
    direction: ConnDirection,
}

impl Fixture {
    fn new(direction: ConnDirection, expected: Option<&str>) -> Self {
        let now = UnixTimestamp::from_millis(1000);
        let mut rng = StdRng::seed_from_u64(42);
        let mut loader = SamodLoader::new(PeerId::from("local"));
        let mut hub = loop {
            match loader.step(&mut rng, now) {
                LoaderState::Loaded(hub) => break *hub,
                LoaderState::NeedIo(tasks) => {
                    for task in tasks {
                        let payload = match task.action {
                            StorageTask::Load { .. } => StorageResult::Load { value: None },
                            StorageTask::LoadRange { .. } => StorageResult::LoadRange {
                                values: HashMap::new(),
                            },
                            StorageTask::Put { .. } => StorageResult::Put,
                            StorageTask::Delete { .. } => StorageResult::Delete,
                        };
                        loader.provide_io_result(IoResult {
                            task_id: task.task_id,
                            payload,
                        });
                    }
                }
            }
        };
        let url = "ws://example.com".parse().unwrap();
        let registration = match direction {
            ConnDirection::Incoming => HubEvent::add_listener(ListenerConfig { url }),
            ConnDirection::Outgoing => HubEvent::add_dialer(DialerConfig {
                url,
                backoff: BackoffConfig::default(),
            }),
        };
        let mut results = hub.handle_event(&mut rng, now, registration.event);
        let expected = expected.map(PeerId::from);
        let (owner, create) = match results
            .completed_commands
            .remove(&registration.command_id)
            .unwrap()
        {
            CommandResult::AddDialer { dialer_id } => (
                ConnectionOwner::Dialer(dialer_id),
                HubEvent::create_dialer_connection(dialer_id, expected),
            ),
            CommandResult::AddListener { listener_id } => (
                ConnectionOwner::Listener(listener_id),
                HubEvent::create_listener_connection(listener_id, expected),
            ),
            other => panic!("unexpected registration: {other:?}"),
        };
        let mut results = hub.handle_event(&mut rng, now, create.event);
        let CommandResult::CreateConnection { connection_id } = results
            .completed_commands
            .remove(&create.command_id)
            .unwrap()
        else {
            panic!("connection creation failed");
        };
        Self {
            hub,
            rng,
            now,
            connection: connection_id,
            owner,
            direction,
        }
    }

    fn event(&mut self, event: HubEvent) -> HubResults {
        self.hub.handle_event(&mut self.rng, self.now, event)
    }

    fn receive(&mut self, msg: WireMessage) -> HubResults {
        self.event(HubEvent::receive(self.connection, msg.encode()).event)
    }

    fn handshake(&mut self, sender: &str) -> HubResults {
        let msg = match self.direction {
            ConnDirection::Incoming => WireMessage::Join {
                sender_id: sender.into(),
                supported_protocol_versions: vec!["1".into()],
                metadata: None,
            },
            ConnDirection::Outgoing => WireMessage::Peer {
                sender_id: sender.into(),
                target_id: "local".into(),
                selected_protocol_version: "1".into(),
                metadata: None,
            },
        };
        self.receive(msg)
    }

    fn assert_disconnected(&self, results: &HubResults, reason: &str) {
        assert!(self.hub.connections().is_empty());
        assert!(self.hub.established_peers().is_empty());
        assert_eq!(results.connections_count, 0);
        assert_eq!(results.new_tasks.iter().filter(|task| matches!(
            task.action, HubIoAction::Disconnect { connection_id } if connection_id == self.connection
        )).count(), 1);
        assert!(results.connection_events.iter().any(|event| matches!(
            event, ConnectionEvent::ConnectionFailed { connection_id, owner, error }
                if *connection_id == self.connection && *owner == self.owner && error.contains(reason)
        )));
        assert!(
            !results
                .connection_events
                .iter()
                .any(|event| matches!(event, ConnectionEvent::HandshakeCompleted { .. }))
        );
        assert!(results.spawn_actors.is_empty());
        assert!(
            !results
                .actor_messages
                .iter()
                .any(|(_, msg)| matches!(msg.0, HubToDocMsgPayload::HandleDocMessage { .. }))
        );
    }
}

#[test]
fn handshake_identity_is_pinned_in_both_directions() {
    for direction in [ConnDirection::Incoming, ConnDirection::Outgoing] {
        let mut f = Fixture::new(direction, Some("remote"));
        // Have a document ready to announce: a rejected handshake must never
        // connect it to the untrusted peer.
        f.event(HubEvent::create_document(automerge::Automerge::new()).event);
        let results = f.handshake("impostor");
        f.assert_disconnected(
            &results,
            "peer ID mismatch: expected remote, received impostor",
        );
        assert!(
            !results
                .actor_messages
                .iter()
                .any(|(_, msg)| matches!(msg.0, HubToDocMsgPayload::NewConnection { .. }))
        );
        assert!(results.new_tasks.iter().all(|task| match &task.action {
            HubIoAction::Send { msg, .. } =>
                matches!(WireMessage::decode(msg).unwrap(), WireMessage::Error { .. }),
            HubIoAction::Disconnect { .. } => true,
        }));
    }
}

#[test]
fn matching_and_unpinned_handshakes_succeed() {
    for direction in [ConnDirection::Incoming, ConnDirection::Outgoing] {
        for expected in [Some("remote"), None] {
            let mut f = Fixture::new(direction, expected);
            let results = f.handshake("remote");
            assert_eq!(
                f.hub.established_peers(),
                vec![(f.connection, "remote".into())]
            );
            assert!(
                results
                    .connection_events
                    .iter()
                    .any(|event| matches!(event, ConnectionEvent::HandshakeCompleted { .. }))
            );
            assert!(
                !results
                    .new_tasks
                    .iter()
                    .any(|task| matches!(task.action, HubIoAction::Disconnect { .. }))
            );
        }
    }
}

fn direct_messages(sender: &str, target: &str) -> Vec<WireMessage> {
    let sender_id = PeerId::from(sender);
    let target_id = PeerId::from(target);
    let document_id = DocumentId::new(&mut StdRng::seed_from_u64(7));
    vec![
        WireMessage::Request {
            sender_id: sender_id.clone(),
            target_id: target_id.clone(),
            document_id: document_id.clone(),
            data: vec![],
        },
        WireMessage::Sync {
            sender_id: sender_id.clone(),
            target_id: target_id.clone(),
            document_id: document_id.clone(),
            data: vec![],
        },
        WireMessage::DocUnavailable {
            sender_id: sender_id.clone(),
            target_id: target_id.clone(),
            document_id: document_id.clone(),
        },
        WireMessage::RemoteHeadsChanged {
            sender_id: sender_id.clone(),
            target_id: target_id.clone(),
            document_id,
            new_heads: HashMap::new(),
        },
        WireMessage::RemoteSubscriptionChange {
            sender_id,
            target_id,
            add: None,
            remove: vec![],
        },
    ]
}

#[test]
fn direct_messages_cannot_change_sender_even_without_pinning() {
    let mut messages = direct_messages("impostor", "local");
    messages.push(WireMessage::Leave {
        sender_id: "impostor".into(),
    });
    for expected in [None, Some("remote")] {
        for msg in &messages {
            let mut f = Fixture::new(ConnDirection::Incoming, expected);
            f.handshake("remote");
            let results = f.receive(msg.clone());
            f.assert_disconnected(&results, "peer ID mismatch");
        }
    }
}

#[test]
fn correctly_addressed_direct_messages_are_accepted() {
    for msg in direct_messages("remote", "local") {
        let mut f = Fixture::new(ConnDirection::Incoming, Some("remote"));
        f.handshake("remote");
        let results = f.receive(msg);
        assert_eq!(results.connections_count, 1);
        assert!(
            !results
                .connection_events
                .iter()
                .any(|event| matches!(event, ConnectionEvent::ConnectionFailed { .. }))
        );
    }
}

#[test]
fn peer_handshake_must_target_us() {
    let mut f = Fixture::new(ConnDirection::Outgoing, Some("remote"));
    let results = f.receive(WireMessage::Peer {
        sender_id: "remote".into(),
        target_id: "someone-else".into(),
        selected_protocol_version: "1".into(),
        metadata: None,
    });
    f.assert_disconnected(&results, "target peer ID mismatch");
}

fn ephemeral(target: &str) -> WireMessage {
    WireMessage::Ephemeral {
        sender_id: "original-author".into(),
        target_id: target.into(),
        count: 1,
        session_id: "original-session".into(),
        document_id: DocumentId::new(&mut StdRng::seed_from_u64(7)),
        data: vec![1, 2, 3],
    }
}

#[test]
fn established_messages_must_target_us() {
    let mut messages = direct_messages("remote", "someone-else");
    messages.push(ephemeral("someone-else"));
    for msg in messages {
        let mut f = Fixture::new(ConnDirection::Incoming, Some("remote"));
        f.handshake("remote");
        let results = f.receive(msg);
        f.assert_disconnected(&results, "target peer ID mismatch");
    }
}

#[test]
fn ephemeral_gossip_can_preserve_another_peers_sender_id() {
    let mut f = Fixture::new(ConnDirection::Incoming, Some("remote"));
    f.handshake("remote");
    let results = f.receive(ephemeral("local"));
    assert_eq!(results.connections_count, 1);
    assert_eq!(results.spawn_actors.len(), 1);
    assert!(
        !results
            .connection_events
            .iter()
            .any(|event| matches!(event, ConnectionEvent::ConnectionFailed { .. }))
    );
}

#[test]
fn rejection_notifies_existing_document_actors() {
    let mut f = Fixture::new(ConnDirection::Incoming, Some("remote"));
    let created = f.event(HubEvent::create_document(automerge::Automerge::new()).event);
    let actor_id = created.spawn_actors[0].actor_id();
    f.handshake("remote");
    let results = f.receive(WireMessage::Leave {
        sender_id: "impostor".into(),
    });
    f.assert_disconnected(&results, "peer ID mismatch");
    assert!(results.actor_messages.iter().any(|(actor, msg)| {
        *actor == actor_id && matches!(msg.0, HubToDocMsgPayload::ConnectionClosed { connection_id } if connection_id == f.connection)
    }));
}

#[test]
fn rejected_dialer_handshake_retries_without_resetting_backoff() {
    let mut f = Fixture::new(ConnDirection::Outgoing, Some("remote"));
    let ConnectionOwner::Dialer(dialer_id) = f.owner else {
        unreachable!()
    };
    let results = f.handshake("impostor");
    f.assert_disconnected(&results, "peer ID mismatch");
    assert_eq!(f.hub.dialer_attempt(dialer_id), Some(1));
    // Duplicate transport loss notifications must not advance backoff twice.
    f.event(HubEvent::connection_lost(f.connection));
    assert_eq!(f.hub.dialer_attempt(dialer_id), Some(1));
    f.now += Duration::from_secs(120);
    let results = f.event(HubEvent::tick());
    assert_eq!(results.dial_requests.len(), 1);
    assert_eq!(results.dial_requests[0].dialer_id, dialer_id);

    let create = HubEvent::create_dialer_connection(dialer_id, Some("remote".into()));
    let mut results = f.event(create.event);
    let CommandResult::CreateConnection { connection_id } = results
        .completed_commands
        .remove(&create.command_id)
        .unwrap()
    else {
        panic!("connection creation failed");
    };
    f.connection = connection_id;
    let results = f.handshake("impostor");
    f.assert_disconnected(&results, "peer ID mismatch");
    assert_eq!(f.hub.dialer_attempt(dialer_id), Some(2));
}

#[test]
fn protocol_and_decode_errors_really_disconnect() {
    for malformed in [false, true] {
        let mut f = Fixture::new(ConnDirection::Outgoing, None);
        let results = if malformed {
            f.event(HubEvent::receive(f.connection, vec![0xff]).event)
        } else {
            // A Join is invalid when waiting for a Peer reply.
            f.receive(WireMessage::Join {
                sender_id: "remote".into(),
                supported_protocol_versions: vec!["1".into()],
                metadata: None,
            })
        };
        f.assert_disconnected(
            &results,
            if malformed {
                "decode error"
            } else {
                "expected a peer message"
            },
        );
        let ConnectionOwner::Dialer(dialer_id) = f.owner else {
            unreachable!()
        };
        assert_eq!(f.hub.dialer_attempt(dialer_id), Some(1));
    }
}
