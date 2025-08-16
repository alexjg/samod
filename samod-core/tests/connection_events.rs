use automerge::{AutomergeError, transaction::Transactable};
use samod_core::network::{ConnectionEvent, ConnectionState};
use samod_test_harness::{Network, RunningDocIds};

fn init_logging() {
    let _ = tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .try_init();
}

#[test]
fn handshake_is_emitted() {
    init_logging();

    let mut network = Network::new();

    let alice = network.create_samod("alice");
    let alice_peer_id = network.samod(&alice).peer_id();
    let bob = network.create_samod("bob");
    let bob_peer_id = network.samod(&bob).peer_id();

    // First, check we get a handshakecomplete
    network.connect(alice, bob);
    network.run_until_quiescent();

    assert!(
        network
            .samod(&alice)
            .connection_events()
            .iter()
            .any(|e| match e {
                ConnectionEvent::HandshakeCompleted { peer_info, .. } =>
                    peer_info.peer_id == bob_peer_id,
                _ => false,
            })
    );

    assert!(
        network
            .samod(&bob)
            .connection_events()
            .iter()
            .any(|e| match e {
                ConnectionEvent::HandshakeCompleted { peer_info, .. } =>
                    peer_info.peer_id == alice_peer_id,
                _ => false,
            })
    );
}

#[test]
fn timestamps_updated_on_receive() {
    init_logging();

    let mut network = Network::new();

    let alice = network.create_samod("alice");
    let alice_peer_id = network.samod(&alice).peer_id();
    let bob = network.create_samod("bob");
    let bob_peer_id = network.samod(&bob).peer_id();

    // First, check we get a handshakecomplete
    network.connect(alice, bob);
    network.run_until_quiescent();

    assert!(
        network
            .samod(&alice)
            .connection_events()
            .iter()
            .any(|e| match e {
                ConnectionEvent::StateChanged { new_state, .. } => {
                    match &new_state.state {
                        ConnectionState::Connected { their_peer_id }
                            if their_peer_id == &bob_peer_id =>
                        {
                            new_state.last_received.is_some() && new_state.last_sent.is_some()
                        }
                        _ => false,
                    }
                }
                _ => false,
            })
    );

    assert!(
        network
            .samod(&bob)
            .connection_events()
            .iter()
            .any(|e| match e {
                ConnectionEvent::StateChanged { new_state, .. } => {
                    match &new_state.state {
                        ConnectionState::Connected { their_peer_id }
                            if their_peer_id == &alice_peer_id =>
                        {
                            new_state.last_received.is_some() && new_state.last_sent.is_some()
                        }
                        _ => false,
                    }
                }
                _ => false,
            })
    )
}

#[test]
fn doc_conn_state_updated_after_sync() {
    init_logging();

    let mut network = Network::new();

    let alice = network.create_samod("alice");
    let bob = network.create_samod("bob");

    // First, check we get a handshakecomplete
    network.connect(alice, bob);
    network.run_until_quiescent();

    // Now create a document on Alice and sync it to Bob
    let RunningDocIds { doc_id, actor_id } = network.samod(&alice).create_document();

    network.samod(&alice).clear_connection_events();
    network.samod(&bob).clear_connection_events();

    let heads = network
        .samod(&alice)
        .with_document_by_actor(actor_id, |doc| {
            doc.transact::<_, _, AutomergeError>(|tx| {
                tx.put(automerge::ROOT, "foo", "bar")?;
                Ok(())
            })
            .unwrap();
            doc.get_heads()
        })
        .unwrap();

    network.run_until_quiescent();

    let last_update_on_alice = network
        .samod(&alice)
        .connection_events()
        .last()
        .cloned()
        .unwrap();
    let ConnectionEvent::StateChanged {
        connection_id: _,
        new_state,
    } = last_update_on_alice
    else {
        panic!("Expected a StateChanged event, got {last_update_on_alice:?}");
    };
    let doc_state = new_state.docs.get(&doc_id).unwrap();
    assert!(doc_state.last_sent.is_some());
    assert!(doc_state.last_received.is_some());
    assert_eq!(doc_state.last_sent_heads, Some(heads.clone()));

    let last_update_on_bob = network
        .samod(&bob)
        .connection_events()
        .last()
        .cloned()
        .unwrap();
    let ConnectionEvent::StateChanged {
        connection_id: _,
        new_state,
    } = last_update_on_bob
    else {
        panic!("Expected a StateChanged event, got {last_update_on_bob:?}");
    };
    let doc_state = new_state.docs.get(&doc_id).unwrap();
    assert!(doc_state.last_sent.is_some());
    assert!(doc_state.last_received.is_some());
    assert_eq!(doc_state.last_sent_heads, Some(heads));
}
