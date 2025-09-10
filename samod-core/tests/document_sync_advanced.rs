use automerge::{AutomergeError, ReadDoc, transaction::Transactable};
use samod_core::network::ConnectionEvent;
use samod_test_harness::{Network, RunningDocIds};

fn init_logging() {
    let _ = tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .try_init();
}

#[test]
fn find_after_create() {
    init_logging();

    let mut network = Network::new();

    let alice = network.create_samod("Alice_Original");
    // Alice creates a document
    let RunningDocIds { doc_id, actor_id } = network.samod(&alice).create_document();
    // Add data to Alice's document
    network
        .samod(&alice)
        .with_document_by_actor(actor_id, |doc| {
            let mut tx = doc.transaction();
            tx.put(automerge::ROOT, "foo", "bar").unwrap();
            tx.commit();
        })
        .expect("with_document should succeed");

    network.run_until_quiescent();

    let alice_storage = network.samod(&alice).storage().clone();
    let alice2 = network.create_samod_with_storage("Alice_Second", alice_storage);
    // now, run find again
    let actor_id2 = network.samod(&alice2).find_document(&doc_id).unwrap();

    let result = {
        network
            .samod(&alice2)
            .with_document_by_actor(actor_id2, |doc| {
                doc.get(automerge::ROOT, "foo")
                    .unwrap()
                    .map(|(value, _)| match value {
                        automerge::Value::Scalar(s) => match s.as_ref() {
                            automerge::ScalarValue::Str(string) => string.to_string(),
                            _ => s.to_string(),
                        },
                        _ => value.to_string(),
                    })
                    .unwrap_or_default()
            })
            .expect("with_document should succeed")
    };
    assert_eq!(result, "bar");
}

#[test]
fn three_peer_chain_sync() {
    init_logging();

    let mut network = Network::new();

    // Create three peers: Alice, Bob, and Charlie
    let alice = network.create_samod("Alice");
    let bob = network.create_samod("Bob");
    let charlie = network.create_samod("Charlie");

    // Connect them in a chain: Alice <-> Bob <-> Charlie
    network.connect(alice, bob);
    network.connect(bob, charlie);

    // Run until handshakes complete
    network.run_until_quiescent();

    // Verify all handshakes completed
    for (name, peer_id) in [("Alice", alice), ("Bob", bob), ("Charlie", charlie)] {
        let events = network.samod(&peer_id).connection_events();
        let handshake_completed = events
            .iter()
            .any(|event| matches!(event, ConnectionEvent::HandshakeCompleted { .. }));
        assert!(handshake_completed, "{name}'s handshake should complete");
    }

    // Charlie creates a document
    let RunningDocIds { doc_id, actor_id } = network.samod(&charlie).create_document();

    // Add a change to Charlie's document
    let result = network
        .samod(&charlie)
        .with_document_by_actor(actor_id, |doc| {
            let mut tx = doc.transaction();
            tx.put(automerge::ROOT, "creator", "charlie").unwrap();
            tx.put(automerge::ROOT, "message", "hello from charlie")
                .unwrap();
            tx.commit();
            "change_applied"
        })
        .unwrap();

    assert_eq!(result, "change_applied");

    // Run here so that charlie has a chance to announce to Bob. Otherwise what can happen
    // is that the request from bob and the requeset from alice cross in the air and so
    // alice considers the document unavailable (because she doesn't have it in storage,
    // and is only connected to bob, who just requested it from her)
    network.run_until_quiescent();

    let alice_actor_id = network.samod(&alice).find_document(&doc_id);

    // Verify Alice found the document through the chain
    assert!(
        alice_actor_id.is_some(),
        "Alice should find the document through Bob from Charlie"
    );
    let alice_actor_id = alice_actor_id.unwrap();

    // Verify Alice's document contains the expected data
    let verification_result = network
        .samod(&alice)
        .with_document_by_actor(alice_actor_id, |doc| {
            // Check the document content
            let creator = doc
                .get(automerge::ROOT, "creator")
                .unwrap()
                .map(|(value, _)| value.into_string().unwrap())
                .unwrap_or_default();
            let message = doc
                .get(automerge::ROOT, "message")
                .unwrap()
                .map(|(value, _)| value.into_string().unwrap())
                .unwrap_or_default();

            (creator, message)
        })
        .expect("with_document should succeed");

    assert_eq!(verification_result.0, "charlie");
    assert_eq!(verification_result.1, "hello from charlie");

    let bob_actor_id = network.samod(&bob).find_document(&doc_id);

    assert!(bob_actor_id.is_some(), "Bob should also have the document");
}

#[test]
fn document_persistence_across_restart() {
    init_logging();

    let mut network = Network::new();

    // Create Alice and Bob (but don't connect them yet)
    let alice_original = network.create_samod("Alice_Original");
    let bob = network.create_samod("Bob");

    // Alice creates a document
    let RunningDocIds { doc_id, actor_id } = network.samod(&alice_original).create_document();

    // Add data to Alice's document
    let result = {
        network
            .samod(&alice_original)
            .with_document_by_actor(actor_id, |doc| {
                let mut tx = doc.transaction();
                tx.put(automerge::ROOT, "persistent_data", "survives_restart")
                    .unwrap();
                tx.commit();
                "data_added"
            })
            .expect("with_document should succeed")
    };

    assert_eq!(result, "data_added");

    // Get Alice's storage to simulate restart
    let alice_storage = network.samod(&alice_original).storage().clone();

    // Create a new Alice instance with the same storage (simulating restart)
    let alice_restarted = network.create_samod_with_storage("Alice_Restarted", alice_storage);

    // Now connect the restarted Alice to Bob
    network.connect(alice_restarted, bob);
    network.run_until_quiescent();

    // Verify handshake completed
    let alice_events = network.samod(&alice_restarted).connection_events();
    let bob_events = network.samod(&bob).connection_events();

    let alice_handshake_completed = alice_events
        .iter()
        .any(|event| matches!(event, ConnectionEvent::HandshakeCompleted { .. }));
    let bob_handshake_completed = bob_events
        .iter()
        .any(|event| matches!(event, ConnectionEvent::HandshakeCompleted { .. }));

    assert!(
        alice_handshake_completed,
        "Restarted Alice's handshake should complete"
    );
    assert!(bob_handshake_completed, "Bob's handshake should complete");

    let bob_actor_id = network
        .samod(&bob)
        .find_document(&doc_id)
        .expect("document should be found on bob");

    // Verify the document data persisted correctly
    let verification_result = network
        .samod(&bob)
        .with_document_by_actor(bob_actor_id, |doc| {
            println!("🔍 Bob's document keys: {:?}", doc.keys(automerge::ROOT));

            doc.get(automerge::ROOT, "persistent_data")
                .unwrap()
                .map(|(value, _)| match value {
                    automerge::Value::Scalar(s) => match s.as_ref() {
                        automerge::ScalarValue::Str(string) => string.to_string(),
                        _ => s.to_string(),
                    },
                    _ => value.to_string(),
                })
                .unwrap_or_default()
        })
        .expect("with_document should succeed");

    assert_eq!(verification_result, "survives_restart");
}

#[test]
fn unavailable_document_multiple_peers() {
    init_logging();

    let mut network = Network::new();

    // Create three peers and connect them all to each other
    let alice = network.create_samod("Alice");
    let bob = network.create_samod("Bob");
    let charlie = network.create_samod("Charlie");

    network.connect(alice, bob);
    network.connect(alice, charlie);
    network.connect(bob, charlie);

    // Run until handshakes complete
    network.run_until_quiescent();
    println!("✅ All three peers connected to each other");

    // Request a non-existent document from Alice
    let fake_doc_id = samod_core::DocumentId::new(&mut rand::rng());
    let alice_result = network.samod(&alice).find_document(&fake_doc_id);

    assert!(alice_result.is_none(), "Document should not be found");
}

#[test]
fn unavailable_document_single_peer() {
    init_logging();

    let mut network = Network::new();

    // Create a single peer
    let alice = network.create_samod("Alice");

    // Request a non-existent document
    let fake_doc_id = samod_core::DocumentId::new(&mut rand::rng());
    let alice_result = network.samod(&alice).find_document(&fake_doc_id);

    assert!(alice_result.is_none(), "Document should not be found");
}

#[test]
fn request_document_before_connection() {
    init_logging();

    let mut network = Network::new();

    // Create Alice and Bob but don't connect them yet
    let alice = network.create_samod("Alice");
    let bob = network.create_samod("Bob");

    // Alice creates a document
    let RunningDocIds { doc_id, actor_id } = network.samod(&alice).create_document();

    // Add data to Alice's document
    let result = network
        .samod(&alice)
        .with_document_by_actor(actor_id, |doc| {
            let mut tx = doc.transaction();
            tx.put(
                automerge::ROOT,
                "delayed_sync",
                "should_work_after_connection",
            )
            .unwrap();
            tx.commit();
            "data_added"
        })
        .expect("with document should succeed");

    assert_eq!(result, "data_added");

    // Bob requests the document before being connected to Alice
    let bob_result = network.samod(&bob).find_document(&doc_id);
    assert!(
        bob_result.is_none(),
        "document should not be found before connection"
    );

    // Now connect Alice and Bob
    network.connect(alice, bob);
    network.run_until_quiescent();

    // Verify handshake completed
    let alice_events = network.samod(&alice).connection_events();
    let bob_events = network.samod(&bob).connection_events();
    let alice_handshake_completed = alice_events
        .iter()
        .any(|event| matches!(event, ConnectionEvent::HandshakeCompleted { .. }));
    let bob_handshake_completed = bob_events
        .iter()
        .any(|event| matches!(event, ConnectionEvent::HandshakeCompleted { .. }));
    assert!(
        alice_handshake_completed,
        "Alice's handshake should complete"
    );
    assert!(bob_handshake_completed, "Bob's handshake should complete");

    // Now find again
    let bob_actor_id = network
        .samod(&bob)
        .find_document(&doc_id)
        .expect("document should be found on bob after connection");

    // Verify the document data
    let verification_result = network
        .samod(&bob)
        .with_document_by_actor(bob_actor_id, |doc| {
            doc.get(automerge::ROOT, "delayed_sync")
                .unwrap()
                .map(|(value, _)| match value {
                    automerge::Value::Scalar(s) => match s.as_ref() {
                        automerge::ScalarValue::Str(string) => string.to_string(),
                        _ => s.to_string(),
                    },
                    _ => value.to_string(),
                })
                .unwrap_or_default()
        })
        .expect("with_document should succeed");

    assert_eq!(verification_result, "should_work_after_connection");
}

#[test]
fn peer_with_announce_policy_set_to_true_should_announce_to_peers() {
    init_logging();

    let mut network = Network::new();

    let alice = network.create_samod("Alice");
    let bob = network.create_samod("Bob");

    network.connect(alice, bob);
    network.run_until_quiescent();

    // Now create a documnt on alice
    let RunningDocIds { doc_id, actor_id } = network.samod(&alice).create_document();
    network
        .samod(&alice)
        .with_document_by_actor(actor_id, |doc| {
            doc.transact::<_, _, AutomergeError>(|tx| {
                tx.put(automerge::ROOT, "foo", "bar")?;
                Ok(())
            })
            .unwrap()
        })
        .unwrap();

    network.run_until_quiescent();

    // Now disconnect the two peers
    network.disconnect(alice, bob);

    // Now, attempting to find the document on bob should succeed because Alice was set
    // to announce the document to this peer
    assert!(network.samod(&bob).find_document(&doc_id).is_some());
}

#[test]
fn peer_with_announce_policy_set_to_false_does_not_announce() {
    init_logging();

    let mut network = Network::new();

    let alice = network.create_samod("Alice");
    network
        .samod(&alice)
        .set_announce_policy(Box::new(|_doc_id, _peer_id| false));
    let bob = network.create_samod("Bob");

    network.connect(alice, bob);
    network.run_until_quiescent();

    // Now create a documnt on alice
    let RunningDocIds { doc_id, actor_id } = network.samod(&alice).create_document();
    network
        .samod(&alice)
        .with_document_by_actor(actor_id, |doc| {
            doc.transact::<_, _, AutomergeError>(|tx| {
                tx.put(automerge::ROOT, "foo", "bar")?;
                Ok(())
            })
            .unwrap()
        })
        .unwrap();

    network.run_until_quiescent();

    // Now disconnect the two peers
    network.disconnect(alice, bob);

    // Now, attempting to find the document on bob should fail because Alice was set
    // to not announce the document
    assert!(network.samod(&bob).find_document(&doc_id).is_none());
}

#[test]
fn peer_with_announce_policy_set_to_false_does_not_request() {
    init_logging();

    let mut network = Network::new();

    let alice = network.create_samod("Alice");
    network
        .samod(&alice)
        .set_announce_policy(Box::new(|_doc_id, _peer_id| false));
    let bob = network.create_samod("Bob");
    network
        .samod(&bob)
        .set_announce_policy(Box::new(|_, _| false));

    network.connect(alice, bob);
    network.run_until_quiescent();

    // Now create a documnt on alice
    let RunningDocIds { doc_id, actor_id } = network.samod(&alice).create_document();
    network
        .samod(&alice)
        .with_document_by_actor(actor_id, |doc| {
            doc.transact::<_, _, AutomergeError>(|tx| {
                tx.put(automerge::ROOT, "foo", "bar")?;
                Ok(())
            })
            .unwrap()
        })
        .unwrap();

    network.run_until_quiescent();

    // Now, attempting to find the document on bob should fail because bob is set
    // to not announce the document and requesting from Alice is equivalent to
    // announcing
    assert!(network.samod(&bob).find_document(&doc_id).is_none());
}

#[test]
fn sync_while_requesting() {
    // Say we have three peers connected like this:
    //
    // alice <-> bob <-> charlie <-> derek
    //
    // Alice is configured to announce everything to bob. The scenario this
    // tests exercises is when Alice creates a document and Derek queries
    // for it before the sync from Alice to Bob to Charlie has completed. There
    // was a bug where the request handling logic meant that Derek would not
    // find the document. The buggy request logic was something like this:
    //
    // "When you receive a sync message for a document not in storage, send
    // a request to every peer you are connected to for the document"
    //
    // This resulted in this sequence of events:
    //
    // * Alice announces document to bob
    // * Bob begins syncing with Alice
    // * Bob also sends a request to Charlie for the document
    // * The `find` call is issued to Derek
    // * Derek sends a request to Charlie
    //
    // At this point Charlie has received a request from Bob and a request from
    // Derek, from Charlies perspective no-one has the document so he returns a
    // not-available response to Derek.

    init_logging();
    let mut network = Network::new();
    let alice = network.create_samod("alice");
    let bob = network.create_samod("bob");
    let charlie = network.create_samod("charlie");
    let derek = network.create_samod("derek");

    network.connect(alice, bob);
    network.connect(bob, charlie);
    network.connect(charlie, derek);

    network.run_until_quiescent();

    // Create the document on alice
    let RunningDocIds { doc_id, .. } = network.samod(&alice).create_document();

    let find_command = network.samod(&derek).begin_find_document(&doc_id);

    network.run_until_quiescent();

    let _ = network
        .samod(&derek)
        .check_find_document_result(find_command)
        .expect("error running find command")
        .expect("derek should have the doc");
}
