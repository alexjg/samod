use samod_test_harness::{Connected, Network, RunningDocIds};

#[test]
fn peer_doc_state_changes_emitted_on_sync() {
    let mut network = Network::new();

    let alice = network.create_samod("alice");
    let bob = network.create_samod("bob");

    let Connected {
        left: bob_on_alice, ..
    } = network.connect(alice, bob);

    let RunningDocIds {
        doc_id,
        actor_id: _alice_actor,
    } = network.samod(&alice).create_document();

    // So far nothing should have happened
    assert!(network.samod(&alice).peer_states(&doc_id).is_empty());

    network.run_until_quiescent();

    let mut changes_on_alice = network.samod(&alice).peer_state_changes(&doc_id).to_vec();
    assert!(!changes_on_alice.is_empty());

    let last_changes = changes_on_alice.pop().unwrap();
    let last_bob_changes = last_changes
        .get(&bob_on_alice)
        .expect("there should be a last change for Bob");
    assert!(last_bob_changes.last_acked_heads.is_some());
    assert!(last_bob_changes.last_sent.is_some());
    let alice_local_heads = network.samod(&alice).document(&doc_id).unwrap().get_heads();
    assert_eq!(last_bob_changes.shared_heads, Some(alice_local_heads));
}
