use automerge::{ReadDoc, transaction::Transactable};
use samod_test_harness::{Network, RunningDocIds};

fn init_logging() {
    let _ = tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .try_init();
}

#[test]
fn basic_create_and_find_change() {
    init_logging();
    let mut network = Network::new();
    let bob = network.create_samod("Bob");

    // Create a document and get its IDs
    let RunningDocIds { doc_id, actor_id } = network.samod(&bob).create_document();

    // Verify that find returns the same actor for the created document
    let found_actor_id = network.samod(&bob).find_document(&doc_id);
    assert_eq!(found_actor_id, Some(actor_id));

    // Test that we can access and modify the document
    network
        .samod(&bob)
        .with_document_by_actor(actor_id, |doc| {
            // Make a change to the document
            let mut tx = doc.transaction();
            tx.put(automerge::ROOT, "test_key", "test_value").unwrap();
            tx.commit();
        })
        .unwrap();

    let verify_result = network
        .samod(&bob)
        .with_document_by_actor(actor_id, |doc| {
            // Verify the document contains our changes within the same instance
            let test_value = doc.get(automerge::ROOT, "test_key").unwrap();
            test_value.and_then(|(value, _)| {
                if let automerge::Value::Scalar(s) = value {
                    s.to_string()
                        .strip_prefix('"')
                        .and_then(|s| s.strip_suffix('"'))
                        .map(|s| s.to_string())
                } else {
                    None
                }
            })
        })
        .expect("with_document should succeed");

    assert_eq!(verify_result, Some("test_value".to_string()));
}
