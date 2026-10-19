use std::collections::HashMap;

use automerge::{Automerge, AutomergeError, ChangeHash, ROOT, transaction::Transactable};
use samod_core::{CompactionHash, DocumentId, StorageKey};
use samod_test_harness::Network;

fn init_logging() {
    let _ = tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .try_init();
}

fn changes(n: usize) -> (Automerge, Vec<Vec<u8>>) {
    let mut doc = Automerge::new();
    let mut changes = Vec::new();
    for i in 0..n {
        doc.transact(|tx| {
            tx.put(ROOT, i.to_string(), i as i64)?;
            Ok::<_, AutomergeError>(())
        })
        .unwrap();
        changes.push(doc.get_last_local_change().unwrap().raw_bytes().to_vec());
    }
    (doc, changes)
}

fn load_heads(storage: HashMap<StorageKey, Vec<u8>>, doc_id: &DocumentId) -> Vec<ChangeHash> {
    let mut network = Network::new();
    let alice = network.create_samod_with_storage("alice", storage);
    let actor = network.samod(&alice).find_document(doc_id).unwrap();
    network
        .samod(&alice)
        .with_document_by_actor(actor, |doc| doc.get_heads())
        .unwrap()
}

fn put_incrementals(
    storage: &mut HashMap<StorageKey, Vec<u8>>,
    doc_id: &DocumentId,
    chunks: impl IntoIterator<Item = Vec<u8>>,
) {
    for chunk in chunks {
        let mut name = [0; 32];
        name[..8].copy_from_slice(&(storage.len() as u64).to_le_bytes());
        storage.insert(
            StorageKey::incremental_path(doc_id, ChangeHash(name)),
            chunk,
        );
    }
}

#[test]
fn truncated_incremental_is_skipped_and_the_rest_load() {
    init_logging();

    for with_snapshot in [false, true] {
        let doc_id = DocumentId::new(&mut rand::rng());
        let (doc, mut files) = changes(30);
        let mut storage = HashMap::new();
        if with_snapshot {
            let mut first = Automerge::new();
            first.load_incremental(&files[..10].concat()).unwrap();
            let hash = CompactionHash::new(&first.get_heads());
            storage.insert(StorageKey::snapshot_path(&doc_id, &hash), first.save());
            files.drain(..10);
        }

        let last = files.pop().unwrap();
        let expected = doc.get_changes(&[])[28].hash();
        put_incrementals(&mut storage, &doc_id, files);
        put_incrementals(&mut storage, &doc_id, [last[..last.len() / 2].to_vec()]);

        assert_eq!(
            load_heads(storage, &doc_id),
            vec![expected],
            "with_snapshot: {with_snapshot}"
        );
    }
}

#[test]
fn incremental_holding_several_changes_loads() {
    init_logging();

    let doc_id = DocumentId::new(&mut rand::rng());
    let (doc, files) = changes(20);
    let mut storage = HashMap::new();
    put_incrementals(&mut storage, &doc_id, files[..5].iter().cloned());
    put_incrementals(&mut storage, &doc_id, [files[5..].concat()]);

    assert_eq!(load_heads(storage, &doc_id), doc.get_heads());
}

#[test]
fn truncated_snapshot_falls_back_to_the_incrementals() {
    init_logging();

    let doc_id = DocumentId::new(&mut rand::rng());
    let (doc, files) = changes(30);
    let mut storage = HashMap::new();
    let snapshot = doc.save();
    let hash = CompactionHash::new(&doc.get_heads());
    storage.insert(
        StorageKey::snapshot_path(&doc_id, &hash),
        snapshot[..snapshot.len() / 2].to_vec(),
    );
    put_incrementals(&mut storage, &doc_id, files);

    assert_eq!(load_heads(storage, &doc_id), doc.get_heads());
}

#[test]
fn incremental_that_fails_to_apply_is_skipped_and_the_rest_load() {
    init_logging();

    let doc_id = DocumentId::new(&mut rand::rng());
    let (doc, mut files) = changes(20);
    let mut first = Automerge::new();
    first.load_incremental(&files[..10].concat()).unwrap();
    let hash = CompactionHash::new(&first.get_heads());
    files.drain(..10);
    let mut rival = Automerge::new().with_actor(doc.get_actor().clone());
    rival
        .transact(|tx| {
            tx.put(ROOT, "rival", 1)?;
            Ok::<_, AutomergeError>(())
        })
        .unwrap();
    let rival_change = rival.get_last_local_change().unwrap().raw_bytes().to_vec();
    let mut storage = HashMap::from([(StorageKey::snapshot_path(&doc_id, &hash), first.save())]);
    put_incrementals(&mut storage, &doc_id, files);
    put_incrementals(&mut storage, &doc_id, [rival_change]);

    assert_eq!(load_heads(storage, &doc_id), doc.get_heads());
}
