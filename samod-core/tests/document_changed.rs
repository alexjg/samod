use automerge::{AutomergeError, ROOT, transaction::Transactable};

use samod_test_harness::{Network, RunningDocIds};

#[test]
fn document_changed_on_receipt_of_message() {
    let mut network = Network::new();

    let alice = network.create_samod("alice");
    let bob = network.create_samod("bob");

    network.connect(alice, bob);

    let RunningDocIds {
        doc_id,
        actor_id: alice_actor,
    } = network.samod(&alice).create_document();

    network.run_until_quiescent();

    let bob_actor = network.samod(&bob).find_document(&doc_id).unwrap();

    network.samod(&bob).pop_doc_changed(bob_actor);

    // Now make a change on alice
    let new_heads = network
        .samod(&alice)
        .with_document_by_actor(alice_actor, |doc| {
            doc.transact::<_, _, AutomergeError>(|tx| {
                tx.put(ROOT, "foo", "bar")?;
                Ok(())
            })
            .unwrap();
            doc.get_heads()
        })
        .unwrap();

    network.run_until_quiescent();

    let change_events = network.samod(&bob).pop_doc_changed(bob_actor);

    assert_eq!(change_events.len(), 1);
    assert_eq!(change_events[0].new_heads, new_heads);
}

#[test]
fn document_changed_after_local_change() {
    let mut network = Network::new();

    let alice = network.create_samod("alice");

    let RunningDocIds {
        doc_id: _,
        actor_id: alice_actor,
    } = network.samod(&alice).create_document();

    // Now make a change on alice
    let new_heads = network
        .samod(&alice)
        .with_document_by_actor(alice_actor, |doc| {
            doc.transact::<_, _, AutomergeError>(|tx| {
                tx.put(ROOT, "foo", "bar")?;
                Ok(())
            })
            .unwrap();
            doc.get_heads()
        })
        .unwrap();

    network.run_until_quiescent();

    let change_events = network.samod(&alice).pop_doc_changed(alice_actor);

    assert_eq!(change_events.len(), 1);
    assert_eq!(change_events[0].new_heads, new_heads);
}
