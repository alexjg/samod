use std::collections::HashMap;

use automerge::{Automerge, AutomergeError, ROOT, transaction::Transactable};
use samod_core::{CompactionHash, DocumentId, StorageKey};
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

#[test]
fn many_changes_compact_but_do_not_delete_existing_snapshot() {
    // This tests a scenario where a bunch of changes have been compacted but
    // the original incremental changes were note deleted for whatever reason.
    // That is, storage looks something like this:
    //
    // /<document ID>/incrementals/<change 1>
    // ..
    // /<document ID>/incrementals/<change 100>
    // /<document ID>/snapshots/<snapshot 1>
    //
    // Where <snapshot 1> is a snapshot of the document made out of the 100 changes
    //
    // The issue that could happen is that on loading the document the repo decides
    // to compact the document (because there are a lot of incremental changes) but
    // the compaction produces the same compacted chunk. This will lead the repo to
    // actually delete the snapshot as it believes it is superceded by the compacted
    // chunk it just wrote.

    // first set up storage
    let mut storage = HashMap::new();
    let doc_id = DocumentId::new(&mut rand::rng());
    let mut doc = Automerge::new();
    for i in 0..100 {
        doc.transact(|tx| {
            tx.put(ROOT, "foo", format!("bar {}", i))?;
            Ok::<_, AutomergeError>(())
        })
        .unwrap();
        let change = doc.get_last_local_change().unwrap();
        storage.insert(
            StorageKey::incremental_path(&doc_id, change.hash()),
            change.raw_bytes().to_vec(),
        );
    }
    // Now write the snapshot
    let snapshot = doc.save();
    let snapshot_hash = CompactionHash::new(&doc.get_heads());
    storage.insert(StorageKey::snapshot_path(&doc_id, &snapshot_hash), snapshot);

    // Now, load an actor with this storage
    let mut network = Network::new();
    let alice = network.create_samod_with_storage("alice", storage);

    // Now look up the document, this will trigger a compactoin
    let _actor_id = network.samod(&alice).find_document(&doc_id).unwrap();

    // Now create another actor with the same storage
    let alice_storage = network.samod(&alice).storage().clone();
    let bob = network.create_samod_with_storage("bob", alice_storage);
    let _actor_id = network
        .samod(&bob)
        .find_document(&doc_id)
        .expect("bob should have the document");
}
