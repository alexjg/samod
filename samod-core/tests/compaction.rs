use std::collections::HashMap;

use automerge::{Automerge, AutomergeError, ROOT, transaction::Transactable};
use samod_core::{CompactionHash, CompactionPolicy, DocumentId, StorageKey};
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

fn incompressible_text(len: usize) -> String {
    use rand::Rng;
    let mut rng = rand::rng();
    (0..len)
        .map(|_| rng.random_range(b'a'..=b'z') as char)
        .collect()
}

fn incrementals_on_disk(storage: &HashMap<StorageKey, Vec<u8>>, doc_id: &DocumentId) -> usize {
    let prefix = StorageKey::incremental_prefix(doc_id);
    storage.keys().filter(|k| prefix.is_prefix_of(k)).count()
}

#[test]
fn small_changes_to_a_large_document_do_not_rewrite_it() {
    init_logging();

    let doc_id = DocumentId::new(&mut rand::rng());
    let mut doc = Automerge::new();
    doc.transact(|tx| {
        tx.put(ROOT, "text", incompressible_text(100_000))?;
        Ok::<_, AutomergeError>(())
    })
    .unwrap();
    let hash = CompactionHash::new(&doc.get_heads());
    let storage = HashMap::from([(StorageKey::snapshot_path(&doc_id, &hash), doc.save())]);

    let mut network = Network::new();
    let alice = network.create_samod_with_storage("alice", storage);
    let actor_id = network.samod(&alice).find_document(&doc_id).unwrap();

    for i in 0..50 {
        network
            .samod(&alice)
            .with_document_by_actor(actor_id, |doc| {
                doc.transact(|tx| {
                    tx.put(ROOT, "counter", i)?;
                    Ok::<_, AutomergeError>(())
                })
                .unwrap();
            })
            .unwrap();
    }
    let snapshot = StorageKey::snapshot_prefix(&doc_id);
    let puts = network.samod(&alice).storage_puts().to_vec();
    assert!(!puts.iter().any(|k| snapshot.is_prefix_of(k)));
    assert_eq!(
        incrementals_on_disk(network.samod(&alice).storage(), &doc_id),
        50
    );
}

#[test]
fn incremental_files_are_capped_by_the_policy() {
    init_logging();

    let mut network = Network::new();
    let alice = network.create_samod_with_compaction(
        "alice",
        CompactionPolicy {
            max_incrementals: 20,
            ..CompactionPolicy::default()
        },
    );
    let RunningDocIds { actor_id, doc_id } = network.samod(&alice).create_document();
    let text = incompressible_text(100_000);
    network
        .samod(&alice)
        .with_document_by_actor(actor_id, |doc| {
            doc.transact(|tx| {
                tx.put(ROOT, "text", text.as_str())?;
                Ok::<_, AutomergeError>(())
            })
            .unwrap();
        })
        .unwrap();

    for i in 0..50 {
        network
            .samod(&alice)
            .with_document_by_actor(actor_id, |doc| {
                doc.transact(|tx| {
                    tx.put(ROOT, "counter", i)?;
                    Ok::<_, AutomergeError>(())
                })
                .unwrap();
            })
            .unwrap();
        let on_disk = incrementals_on_disk(network.samod(&alice).storage(), &doc_id);
        assert!(on_disk <= 20, "{on_disk} incremental files on disk");
    }
    let heads = network
        .samod(&alice)
        .with_document_by_actor(actor_id, |doc| doc.get_heads())
        .unwrap();

    let storage = network.samod(&alice).storage().clone();
    let alice2 = network.create_samod_with_storage("alice2", storage);
    let actor = network.samod(&alice2).find_document(&doc_id).unwrap();
    let reloaded = network
        .samod(&alice2)
        .with_document_by_actor(actor, |doc| doc.get_heads())
        .unwrap();
    assert_eq!(heads, reloaded);
}

#[test]
fn loading_a_document_does_not_rewrite_it() {
    init_logging();

    let mut network = Network::new();
    let alice = network.create_samod("alice");
    let RunningDocIds { actor_id, doc_id } = network.samod(&alice).create_document();

    network
        .samod(&alice)
        .with_document_by_actor(actor_id, |doc| {
            for i in 0..20 {
                doc.transact(|tx| {
                    tx.put(ROOT, i.to_string(), i)?;
                    Ok::<_, AutomergeError>(())
                })
                .unwrap();
            }
        })
        .unwrap();
    let storage = network.samod(&alice).storage().clone();

    let alice2 = network.create_samod_with_storage("alice2", storage);
    let actor = network.samod(&alice2).find_document(&doc_id).unwrap();
    network
        .samod(&alice2)
        .with_document_by_actor(actor, |doc| {
            doc.transact(|tx| {
                tx.put(ROOT, "late", 1)?;
                Ok::<_, AutomergeError>(())
            })
            .unwrap();
        })
        .unwrap();

    let incremental = StorageKey::incremental_prefix(&doc_id);
    let puts = network.samod(&alice2).storage_puts().to_vec();
    assert_eq!(puts.len(), 1, "expected one put, got {puts:?}");
    assert!(incremental.is_prefix_of(&puts[0]));
}

#[test]
fn small_document_is_not_rewritten_on_every_change() {
    init_logging();

    let mut network = Network::new();
    let alice = network.create_samod("alice");
    let RunningDocIds { actor_id, doc_id } = network.samod(&alice).create_document();
    let edit = |network: &mut Network, i: usize| {
        network
            .samod(&alice)
            .with_document_by_actor(actor_id, |doc| {
                doc.transact(|tx| {
                    tx.put(ROOT, "counter", i as i64)?;
                    Ok::<_, AutomergeError>(())
                })
                .unwrap();
            })
            .unwrap();
    };
    let snapshot_puts = |network: &mut Network| {
        let prefix = StorageKey::snapshot_prefix(&doc_id);
        network
            .samod(&alice)
            .storage_puts()
            .iter()
            .filter(|k| prefix.is_prefix_of(k))
            .count()
    };

    let mut i = 0;
    while incrementals_on_disk(network.samod(&alice).storage(), &doc_id) < 10 {
        edit(&mut network, i);
        i += 1;
    }
    assert_eq!(snapshot_puts(&mut network), 0);

    edit(&mut network, i);
    assert_eq!(snapshot_puts(&mut network), 1);
}

#[test]
#[should_panic(expected = "incremental_fraction must be non-negative")]
fn negative_incremental_fraction_is_rejected() {
    let _ = samod_core::SamodLoader::new(samod_core::PeerId::from("alice")).with_compaction_policy(
        CompactionPolicy {
            incremental_fraction: -1.0,
            ..CompactionPolicy::default()
        },
    );
}

#[test]
#[should_panic(expected = "incremental_fraction must be non-negative")]
fn nan_incremental_fraction_is_rejected() {
    let _ = samod_core::SamodLoader::new(samod_core::PeerId::from("alice")).with_compaction_policy(
        CompactionPolicy {
            incremental_fraction: f64::NAN,
            ..CompactionPolicy::default()
        },
    );
}
