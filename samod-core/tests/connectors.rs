use std::time::Duration;

use samod_core::{
    BackoffConfig, CommandResult, DialerConfig, DialerEvent, DialerId, ListenerConfig, ListenerId,
    StorageKey, UnixTimestamp,
    actors::hub::{DispatchedCommand, Hub, HubEvent, HubResults},
    io::{IoResult, StorageResult, StorageTask},
};

/// Helper: create a Hub via the loader.
fn make_hub(name: &str) -> Hub {
    use rand::SeedableRng;
    use samod_core::{LoaderState, PeerId, SamodLoader};
    use std::collections::HashMap;

    let peer_id = PeerId::from_string(name.to_string());
    let mut loader = SamodLoader::new(peer_id);
    let now = UnixTimestamp::from_millis(1000);
    let mut rng = rand::rngs::StdRng::seed_from_u64(42);
    let mut storage: HashMap<StorageKey, Vec<u8>> = HashMap::new();

    loop {
        match loader.step(&mut rng, now) {
            LoaderState::NeedIo(tasks) => {
                for task in tasks {
                    let result = match task.action {
                        StorageTask::Load { ref key } => StorageResult::Load {
                            value: storage.get(key).cloned(),
                        },
                        StorageTask::LoadRange { ref prefix } => StorageResult::LoadRange {
                            values: storage
                                .iter()
                                .filter(|(k, _)| prefix.is_prefix_of(k))
                                .map(|(k, v)| (k.clone(), v.clone()))
                                .collect(),
                        },
                        StorageTask::Put { ref key, ref value } => {
                            storage.insert(key.clone(), value.clone());
                            StorageResult::Put
                        }
                        StorageTask::Delete { ref key } => {
                            storage.remove(key);
                            StorageResult::Delete
                        }
                    };
                    loader.provide_io_result(IoResult {
                        task_id: task.task_id,
                        payload: result,
                    });
                }
            }
            LoaderState::Loaded(hub) => break *hub,
        }
    }
}

fn make_rng() -> rand::rngs::StdRng {
    use rand::SeedableRng;
    rand::rngs::StdRng::seed_from_u64(42)
}

/// Helper: process a hub event.
fn handle_event(
    hub: &mut Hub,
    rng: &mut impl rand::Rng,
    now: UnixTimestamp,
    event: HubEvent,
) -> HubResults {
    hub.handle_event(rng, now, event)
}

/// Helper: add a dialer and return (dialer_id, results).
fn add_dialer(
    hub: &mut Hub,
    rng: &mut impl rand::Rng,
    now: UnixTimestamp,
    config: DialerConfig,
) -> (DialerId, HubResults) {
    let DispatchedCommand { command_id, event } = HubEvent::add_dialer(config);
    let results = handle_event(hub, rng, now, event);

    let dialer_id = results
        .completed_commands
        .iter()
        .find_map(|(cid, result)| {
            if *cid == command_id {
                if let CommandResult::AddDialer { dialer_id } = result {
                    Some(*dialer_id)
                } else {
                    None
                }
            } else {
                None
            }
        })
        .expect("add_dialer should complete immediately");

    (dialer_id, results)
}

/// Helper: add a listener and return (listener_id, results).
fn add_listener(
    hub: &mut Hub,
    rng: &mut impl rand::Rng,
    now: UnixTimestamp,
    config: ListenerConfig,
) -> (ListenerId, HubResults) {
    let DispatchedCommand { command_id, event } = HubEvent::add_listener(config);
    let results = handle_event(hub, rng, now, event);

    let listener_id = results
        .completed_commands
        .iter()
        .find_map(|(cid, result)| {
            if *cid == command_id {
                if let CommandResult::AddListener { listener_id } = result {
                    Some(*listener_id)
                } else {
                    None
                }
            } else {
                None
            }
        })
        .expect("add_listener should complete immediately");

    (listener_id, results)
}

/// Helper: create a connection for a dialer and return the connection_id.
fn create_dialer_connection(
    hub: &mut Hub,
    rng: &mut impl rand::Rng,
    now: UnixTimestamp,
    dialer_id: DialerId,
) -> samod_core::ConnectionId {
    let DispatchedCommand { command_id, event } = HubEvent::create_dialer_connection(dialer_id);
    let results = handle_event(hub, rng, now, event);
    results
        .completed_commands
        .iter()
        .find_map(|(cid, result)| {
            if *cid == command_id {
                if let CommandResult::CreateConnection { connection_id } = result {
                    Some(*connection_id)
                } else {
                    None
                }
            } else {
                None
            }
        })
        .expect("create_dialer_connection should complete immediately")
}

/// Helper: create a connection for a listener and return the connection_id.
fn create_listener_connection(
    hub: &mut Hub,
    rng: &mut impl rand::Rng,
    now: UnixTimestamp,
    listener_id: ListenerId,
) -> samod_core::ConnectionId {
    let DispatchedCommand { command_id, event } = HubEvent::create_listener_connection(listener_id);
    let results = handle_event(hub, rng, now, event);
    results
        .completed_commands
        .iter()
        .find_map(|(cid, result)| {
            if *cid == command_id {
                if let CommandResult::CreateConnection { connection_id } = result {
                    Some(*connection_id)
                } else {
                    None
                }
            } else {
                None
            }
        })
        .expect("create_listener_connection should complete immediately")
}

// =============================================================================
// Tests
// =============================================================================

#[test]
fn dialer_emits_dial_request_on_add() {
    let mut hub = make_hub("alice");
    let mut rng = make_rng();
    let now = UnixTimestamp::from_millis(1000);

    let url = url::Url::parse("wss://sync.example.com").unwrap();
    let config = DialerConfig {
        url: url.clone(),
        backoff: BackoffConfig::default(),
    };

    let (dialer_id, results) = add_dialer(&mut hub, &mut rng, now, config);

    // Should emit exactly one dial request for the dialer
    assert_eq!(results.dial_requests.len(), 1);
    assert_eq!(results.dial_requests[0].dialer_id, dialer_id);
    assert_eq!(results.dial_requests[0].url.as_str(), url.as_str());
}

#[test]
fn listener_does_not_emit_dial_request() {
    let mut hub = make_hub("alice");
    let mut rng = make_rng();
    let now = UnixTimestamp::from_millis(1000);

    let url = url::Url::parse("ws://0.0.0.0:8080").unwrap();
    let config = ListenerConfig { url };

    let (_listener_id, results) = add_listener(&mut hub, &mut rng, now, config);

    assert!(results.dial_requests.is_empty());
}

#[test]
fn dialer_create_connection_transitions_to_connected() {
    let mut hub = make_hub("alice");
    let mut rng = make_rng();
    let now = UnixTimestamp::from_millis(1000);

    let url = url::Url::parse("wss://sync.example.com").unwrap();
    let config = DialerConfig {
        url,
        backoff: BackoffConfig::default(),
    };

    let (dialer_id, _results) = add_dialer(&mut hub, &mut rng, now, config);

    // Atomically create a connection for the dialer (replaces old transport_ready)
    let _connection_id = create_dialer_connection(&mut hub, &mut rng, now, dialer_id);

    // No further dial requests should be emitted (dialer is now connected)
    let results = handle_event(&mut hub, &mut rng, now, HubEvent::tick());
    assert!(results.dial_requests.is_empty());
    assert!(results.dialer_events.is_empty());
}

#[test]
fn dialer_dial_failed_schedules_retry() {
    let mut hub = make_hub("alice");
    let mut rng = make_rng();
    let now = UnixTimestamp::from_millis(1000);

    let url = url::Url::parse("wss://sync.example.com").unwrap();
    let config = DialerConfig {
        url,
        backoff: BackoffConfig {
            initial_delay: Duration::from_millis(100),
            max_delay: Duration::from_secs(30),
            max_retries: None,
        },
    };

    let (dialer_id, _results) = add_dialer(&mut hub, &mut rng, now, config);

    let results = handle_event(
        &mut hub,
        &mut rng,
        now,
        HubEvent::dial_failed(dialer_id, "connection refused".to_string()),
    );

    // Should NOT immediately emit a new dial request (waiting for backoff)
    assert!(results.dial_requests.is_empty());
    // Not failed permanently (unlimited retries)
    assert!(results.dialer_events.is_empty());
}

#[test]
fn dialer_retries_after_tick() {
    let mut hub = make_hub("alice");
    let mut rng = make_rng();
    let now = UnixTimestamp::from_millis(1000);

    let url = url::Url::parse("wss://sync.example.com").unwrap();
    let config = DialerConfig {
        url: url.clone(),
        backoff: BackoffConfig {
            initial_delay: Duration::from_millis(100),
            max_delay: Duration::from_secs(30),
            max_retries: None,
        },
    };

    let (dialer_id, _results) = add_dialer(&mut hub, &mut rng, now, config);

    handle_event(
        &mut hub,
        &mut rng,
        now,
        HubEvent::dial_failed(dialer_id, "connection refused".to_string()),
    );

    // Tick too early
    let early_tick = now + Duration::from_millis(10);
    let results = handle_event(&mut hub, &mut rng, early_tick, HubEvent::tick());
    assert!(results.dial_requests.is_empty());

    // Tick well after backoff
    let late_tick = now + Duration::from_secs(1);
    let results = handle_event(&mut hub, &mut rng, late_tick, HubEvent::tick());
    assert_eq!(results.dial_requests.len(), 1);
    assert_eq!(results.dial_requests[0].dialer_id, dialer_id);
}

#[test]
fn dialer_max_retries_reached() {
    let mut hub = make_hub("alice");
    let mut rng = make_rng();
    let mut now = UnixTimestamp::from_millis(1000);

    let url = url::Url::parse("wss://sync.example.com").unwrap();
    let config = DialerConfig {
        url: url.clone(),
        backoff: BackoffConfig {
            initial_delay: Duration::from_millis(100),
            max_delay: Duration::from_secs(30),
            max_retries: Some(2),
        },
    };

    let (dialer_id, _results) = add_dialer(&mut hub, &mut rng, now, config);

    // Fail attempt 1
    handle_event(
        &mut hub,
        &mut rng,
        now,
        HubEvent::dial_failed(dialer_id, "fail 1".to_string()),
    );

    now += Duration::from_secs(10);
    let results = handle_event(&mut hub, &mut rng, now, HubEvent::tick());
    assert_eq!(
        results.dial_requests.len(),
        1,
        "should retry after first failure"
    );

    // Fail attempt 2
    let results = handle_event(
        &mut hub,
        &mut rng,
        now,
        HubEvent::dial_failed(dialer_id, "fail 2".to_string()),
    );
    assert!(results.dialer_events.is_empty());

    now += Duration::from_secs(10);
    let results = handle_event(&mut hub, &mut rng, now, HubEvent::tick());
    assert_eq!(
        results.dial_requests.len(),
        1,
        "should retry after second failure"
    );

    // Fail attempt 3 - exceeds max_retries of 2
    let results = handle_event(
        &mut hub,
        &mut rng,
        now,
        HubEvent::dial_failed(dialer_id, "fail 3".to_string()),
    );

    assert_eq!(results.dialer_events.len(), 1);
    match &results.dialer_events[0] {
        DialerEvent::MaxRetriesReached {
            dialer_id: did,
            url: event_url,
        } => {
            assert_eq!(*did, dialer_id);
            assert_eq!(event_url.as_str(), url.as_str());
        }
    }

    // Permanently failed - no more retries
    now += Duration::from_secs(100);
    let results = handle_event(&mut hub, &mut rng, now, HubEvent::tick());
    assert!(results.dial_requests.is_empty());
}

#[test]
fn dialer_connection_lost_triggers_backoff() {
    let mut hub = make_hub("alice");
    let mut rng = make_rng();
    let now = UnixTimestamp::from_millis(1000);

    let url = url::Url::parse("wss://sync.example.com").unwrap();
    let config = DialerConfig {
        url,
        backoff: BackoffConfig {
            initial_delay: Duration::from_millis(100),
            max_delay: Duration::from_secs(30),
            max_retries: None,
        },
    };

    let (dialer_id, _results) = add_dialer(&mut hub, &mut rng, now, config);

    let connection_id = create_dialer_connection(&mut hub, &mut rng, now, dialer_id);

    let results = handle_event(
        &mut hub,
        &mut rng,
        now,
        HubEvent::connection_lost(connection_id),
    );

    assert!(results.dial_requests.is_empty());
    assert!(results.dialer_events.is_empty());

    let later = now + Duration::from_secs(1);
    let results = handle_event(&mut hub, &mut rng, later, HubEvent::tick());
    assert_eq!(results.dial_requests.len(), 1);
}

#[test]
fn listener_accepts_multiple_connections() {
    let mut hub = make_hub("alice");
    let mut rng = make_rng();
    let now = UnixTimestamp::from_millis(1000);

    let url = url::Url::parse("ws://0.0.0.0:8080").unwrap();
    let config = ListenerConfig { url };

    let (listener_id, _results) = add_listener(&mut hub, &mut rng, now, config);

    let _conn1 = create_listener_connection(&mut hub, &mut rng, now, listener_id);
    let _conn2 = create_listener_connection(&mut hub, &mut rng, now, listener_id);

    assert_eq!(hub.connections().len(), 2);
}

#[test]
fn listener_connection_lost_does_not_retry() {
    let mut hub = make_hub("alice");
    let mut rng = make_rng();
    let now = UnixTimestamp::from_millis(1000);

    let url = url::Url::parse("ws://0.0.0.0:8080").unwrap();
    let config = ListenerConfig { url };

    let (listener_id, _results) = add_listener(&mut hub, &mut rng, now, config);

    let conn1 = create_listener_connection(&mut hub, &mut rng, now, listener_id);

    let results = handle_event(&mut hub, &mut rng, now, HubEvent::connection_lost(conn1));

    assert!(results.dial_requests.is_empty());
    assert!(results.dialer_events.is_empty());

    let later = now + Duration::from_secs(10);
    let results = handle_event(&mut hub, &mut rng, later, HubEvent::tick());
    assert!(results.dial_requests.is_empty());
}

#[test]
fn remove_dialer_stops_retries() {
    let mut hub = make_hub("alice");
    let mut rng = make_rng();
    let now = UnixTimestamp::from_millis(1000);

    let url = url::Url::parse("wss://sync.example.com").unwrap();
    let config = DialerConfig {
        url,
        backoff: BackoffConfig::default(),
    };

    let (dialer_id, _results) = add_dialer(&mut hub, &mut rng, now, config);

    handle_event(&mut hub, &mut rng, now, HubEvent::remove_dialer(dialer_id));

    let later = now + Duration::from_secs(100);
    let results = handle_event(&mut hub, &mut rng, later, HubEvent::tick());
    assert!(results.dial_requests.is_empty());
}

#[test]
fn backoff_delay_increases_exponentially() {
    let mut hub = make_hub("alice");
    let mut rng = make_rng();
    let now = UnixTimestamp::from_millis(1_000_000);

    let url = url::Url::parse("wss://sync.example.com").unwrap();
    let config = DialerConfig {
        url,
        backoff: BackoffConfig {
            initial_delay: Duration::from_millis(1000),
            max_delay: Duration::from_secs(60),
            max_retries: None,
        },
    };

    let (dialer_id, _results) = add_dialer(&mut hub, &mut rng, now, config);

    let mut retry_triggers = Vec::new();

    for i in 0..5 {
        handle_event(
            &mut hub,
            &mut rng,
            now,
            HubEvent::dial_failed(dialer_id, format!("fail {i}")),
        );

        let mut tick_time = now;
        loop {
            tick_time += Duration::from_millis(100);
            let results = handle_event(&mut hub, &mut rng, tick_time, HubEvent::tick());
            if !results.dial_requests.is_empty() {
                retry_triggers.push(tick_time - now);
                break;
            }
            if (tick_time - now) > Duration::from_secs(120) {
                panic!("retry never triggered for attempt {i}");
            }
        }
    }

    // Later retries should take longer than earlier ones
    assert!(
        retry_triggers[4] > retry_triggers[0],
        "fifth retry delay {:?} should be larger than first {:?}",
        retry_triggers[4],
        retry_triggers[0],
    );
}

#[test]
fn backoff_capped_at_max_delay() {
    let mut hub = make_hub("alice");
    let mut rng = make_rng();
    let now = UnixTimestamp::from_millis(1_000_000);

    let max_delay = Duration::from_secs(2);
    let url = url::Url::parse("wss://sync.example.com").unwrap();
    let config = DialerConfig {
        url,
        backoff: BackoffConfig {
            initial_delay: Duration::from_millis(100),
            max_delay,
            max_retries: None,
        },
    };

    let (dialer_id, _results) = add_dialer(&mut hub, &mut rng, now, config);

    for i in 0..20 {
        handle_event(
            &mut hub,
            &mut rng,
            now,
            HubEvent::dial_failed(dialer_id, format!("fail {i}")),
        );

        let tick_time = now + max_delay + Duration::from_millis(100);
        let results = handle_event(&mut hub, &mut rng, tick_time, HubEvent::tick());
        assert!(
            !results.dial_requests.is_empty(),
            "retry should trigger within max_delay + margin on attempt {i}"
        );
    }
}

#[test]
fn find_listener_for_url() {
    let mut hub = make_hub("alice");
    let mut rng = make_rng();
    let now = UnixTimestamp::from_millis(1000);

    let url1 = url::Url::parse("ws://0.0.0.0:8080").unwrap();
    let url2 = url::Url::parse("ws://0.0.0.0:9090").unwrap();

    assert!(hub.find_listener_for_url(&url1).is_none());

    let config = ListenerConfig { url: url1.clone() };
    let (listener_id, _) = add_listener(&mut hub, &mut rng, now, config);

    assert_eq!(hub.find_listener_for_url(&url1), Some(listener_id));
    assert!(hub.find_listener_for_url(&url2).is_none());
}

#[test]
fn dialer_attempt_tracking() {
    let mut hub = make_hub("alice");
    let mut rng = make_rng();
    let mut now = UnixTimestamp::from_millis(1000);

    let url = url::Url::parse("wss://sync.example.com").unwrap();
    let config = DialerConfig {
        url,
        backoff: BackoffConfig {
            initial_delay: Duration::from_millis(100),
            max_delay: Duration::from_secs(30),
            max_retries: None,
        },
    };

    let (dialer_id, _) = add_dialer(&mut hub, &mut rng, now, config);

    assert_eq!(hub.dialer_attempt(dialer_id), Some(0));

    handle_event(
        &mut hub,
        &mut rng,
        now,
        HubEvent::dial_failed(dialer_id, "fail".to_string()),
    );
    assert_eq!(hub.dialer_attempt(dialer_id), Some(1));

    now += Duration::from_secs(10);
    handle_event(&mut hub, &mut rng, now, HubEvent::tick());

    handle_event(
        &mut hub,
        &mut rng,
        now,
        HubEvent::dial_failed(dialer_id, "fail".to_string()),
    );
    assert_eq!(hub.dialer_attempt(dialer_id), Some(2));
}
