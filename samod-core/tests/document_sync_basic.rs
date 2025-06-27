use automerge::{ReadDoc, transaction::Transactable};
use samod_core::network::ConnectionEvent;
use samod_test_harness::{Network, RunningDocIds};

fn init_logging() {
    let _ = tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .try_init();
}

#[test]
fn basic_document_sync_flow() {
    init_logging();
    let mut network = Network::new();

    // Step 1: Create two peers, Alice and Bob
    let alice = network.create_samod("Alice");
    let bob = network.create_samod("Bob");

    // Step 2: Alice creates a new document and adds a single change to it
    let RunningDocIds { doc_id, actor_id } = network.samod(&alice).create_document();

    // Add a change to Alice's document: set "foo" to "bar"
    network
        .samod(&alice)
        .with_document_by_actor(actor_id, |doc| {
            let mut tx = doc.transaction();
            tx.put(automerge::ROOT, "foo", "bar").unwrap();
            tx.commit();
        })
        .expect("with document should succeed");

    // Step 3: Alice connects to Bob
    network.connect(alice, bob);

    // Run until handshake completes
    network.run_until_quiescent();

    // Verify handshake completed successfully
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

    // Step 4: Bob requests the document (by calling Command::FindDocument)
    // This now requires async peer communication, so we need to handle it differently
    let bob_actor_id = network
        .samod(&bob)
        .find_document(&doc_id)
        .expect("Bob should find Alice's document");

    // Verify Bob's document contains the expected data
    let verification_result = network
        .samod(&bob)
        .with_document_by_actor(bob_actor_id, |doc| {
            // Check if the document has "foo" set to "bar"
            let foo_value = doc.get(automerge::ROOT, "foo").unwrap();
            foo_value
                .map(|(value, _)| match value {
                    automerge::Value::Scalar(s) => match s.as_ref() {
                        automerge::ScalarValue::Str(string) => string == "bar",
                        _ => false,
                    },
                    _ => false,
                })
                .unwrap_or(false)
        })
        .expect("with_document should succeed");

    assert!(
        verification_result,
        "Bob's document should contain Alice's data (foo -> bar)"
    );
}
