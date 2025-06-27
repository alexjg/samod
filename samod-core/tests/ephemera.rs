use std::collections::HashSet;

use samod_test_harness::{Network, RunningDocIds};

fn init_logging() {
    let _ = tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .try_init();
}

#[test]
fn ephemera_smoke() {
    init_logging();

    let mut network = Network::new();

    let alice = network.create_samod("alice");
    let bob = network.create_samod("bob");

    network.connect(alice, bob);

    // Alice creates a document
    let RunningDocIds {
        doc_id,
        actor_id: alice_actor,
    } = network.samod(&alice).create_document();
    network.run_until_quiescent();

    let bob_actor = network.samod(&bob).find_document(&doc_id).unwrap();
    network.samod(&bob).broadcast(bob_actor, vec![1, 2, 3]);

    network.run_until_quiescent();

    let msgs = network.samod(&alice).pop_ephemera(alice_actor);
    assert_eq!(msgs, vec![vec![1, 2, 3]]);
}

#[test]
fn ephemeral_are_forwarded() {
    init_logging();

    let mut network = Network::new();

    let alice = network.create_samod("alice");
    let bob = network.create_samod("bob");
    let charlie = network.create_samod("charlie");

    network.connect(alice, bob);
    network.connect(bob, charlie);

    // Alice creates a document
    let RunningDocIds {
        doc_id,
        actor_id: alice_actor,
    } = network.samod(&alice).create_document();
    network.run_until_quiescent();

    let charlie_actor = network.samod(&charlie).find_document(&doc_id).unwrap();
    network
        .samod(&charlie)
        .broadcast(charlie_actor, vec![1, 2, 3]);

    network.run_until_quiescent();

    let msgs = network.samod(&alice).pop_ephemera(alice_actor);
    assert_eq!(msgs, vec![vec![1, 2, 3]]);
}

#[test]
fn ephemeral_do_not_loop() {
    init_logging();

    let mut network = Network::new();

    let alice = network.create_samod("alice");
    let bob = network.create_samod("bob");
    let charlie = network.create_samod("charlie");

    network.connect(alice, bob);
    network.connect(bob, charlie);
    network.connect(charlie, alice);

    // Alice creates a document
    let RunningDocIds {
        doc_id,
        actor_id: alice_actor,
    } = network.samod(&alice).create_document();
    network.run_until_quiescent();

    let charlie_actor = network.samod(&charlie).find_document(&doc_id).unwrap();
    network
        .samod(&charlie)
        .broadcast(charlie_actor, vec![1, 2, 3]);

    let bob_actor = network.samod(&bob).find_document(&doc_id).unwrap();
    network.samod(&bob).broadcast(bob_actor, vec![1, 2, 3]);

    network.run_until_quiescent();

    // The reason for the complicated assertions below is that which order
    // messages are received in, and how many times, is non-deterministic.
    // However, we know that we should never receive more messages than peers
    // in the network and we should receive at least one and all the messages
    // should be the same (as we only broadcast once)

    let alice_msgs = network.samod(&alice).pop_ephemera(alice_actor);
    assert!(alice_msgs.len() < 3 && !alice_msgs.is_empty());
    assert_eq!(
        alice_msgs.into_iter().collect::<HashSet<_>>(),
        HashSet::from([vec![1, 2, 3]])
    );

    let bob_msgs = network.samod(&bob).pop_ephemera(bob_actor);
    assert!(bob_msgs.len() < 3 && !bob_msgs.is_empty());
    assert_eq!(
        bob_msgs.into_iter().collect::<HashSet<_>>(),
        HashSet::from([vec![1, 2, 3]])
    );

    let charlie_msgs = network.samod(&charlie).pop_ephemera(charlie_actor);
    assert!(charlie_msgs.len() < 3 && !charlie_msgs.is_empty());
    assert_eq!(
        charlie_msgs.into_iter().collect::<HashSet<_>>(),
        HashSet::from([vec![1, 2, 3]])
    );
}
