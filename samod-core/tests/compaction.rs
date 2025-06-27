use automerge::{AutomergeError, ROOT, transaction::Transactable};
use samod_test_harness::{Network, RunningDocIds};

fn init_logging() {
    let _ = tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .try_init();
}

#[test]
fn many_changes_are_compacted() {
    init_logging();

    let mut network = Network::new();
    let alice = network.create_samod("alice");

    // Create a document
    let RunningDocIds { actor_id, doc_id } = network.samod(&alice).create_document();

    // Now make lot's of changes
    for i in 0..100 {
        network
            .samod(&alice)
            .with_document_by_actor(actor_id, |doc| {
                doc.transact(|tx| {
                    tx.put(ROOT, i.to_string(), i)?;
                    Ok::<_, AutomergeError>(())
                })
                .unwrap();
            })
            .unwrap();
    }

    let doc_heads = network
        .samod(&alice)
        .with_document_by_actor(actor_id, |doc| doc.get_heads())
        .unwrap();

    // Now, there should be less than 100 changes in storage due to compaction
    let num_changes = network.samod(&alice).storage().len();
    assert!(
        num_changes < 100,
        "Expected less than 100 changes, found {num_changes}"
    );

    // Reload the document on a new peer and check it is the same
    let alice_storage = network.samod(&alice).storage().clone();
    let alice2 = network.create_samod_with_storage("alice2", alice_storage);
    let alice2_actor = network.samod(&alice2).find_document(&doc_id).unwrap();

    let heads_on_alice2 = network
        .samod(&alice2)
        .with_document_by_actor(alice2_actor, |doc| doc.get_heads())
        .unwrap();

    assert_eq!(doc_heads, heads_on_alice2);
}
