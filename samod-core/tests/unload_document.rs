use automerge::{ReadDoc, transaction::Transactable};
use samod_core::DocumentId;
use samod_test_harness::{Network, RunningDocIds};

fn init_logging() {
    let _ = tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .try_init();
}

#[test]
fn unload_removes_document_and_find_reloads_from_storage() {
    init_logging();
    let mut network = Network::new();
    let alice = network.create_samod("alice");

    let RunningDocIds { doc_id, actor_id } = network.samod(&alice).create_document();
    network
        .samod(&alice)
        .with_document_by_actor(actor_id, |doc| {
            let mut tx = doc.transaction();
            tx.put(automerge::ROOT, "key", "value").unwrap();
            tx.commit();
        })
        .unwrap();
    network.run_until_quiescent();
    assert_eq!(network.samod(&alice).actor_count(), 1);

    network.samod(&alice).unload_document(&doc_id);

    assert_eq!(network.samod(&alice).actor_count(), 0);
    assert!(network.samod(&alice).get_actor(&actor_id).is_none());

    let new_actor_id = network
        .samod(&alice)
        .find_document(&doc_id)
        .expect("document should load back from storage");
    assert_ne!(new_actor_id, actor_id);
    let value = network
        .samod(&alice)
        .with_document_by_actor(new_actor_id, |doc| {
            doc.get(automerge::ROOT, "key")
                .unwrap()
                .map(|(v, _)| v.to_string())
        })
        .unwrap();
    assert_eq!(value.as_deref(), Some("\"value\""));
}

#[test]
fn unload_unknown_document_is_a_noop() {
    init_logging();
    let mut network = Network::new();
    let alice = network.create_samod("alice");
    network.samod(&alice).create_document();

    let mut rng = rand::rng();
    network
        .samod(&alice)
        .unload_document(&DocumentId::new(&mut rng));

    assert_eq!(network.samod(&alice).actor_count(), 1);
}
