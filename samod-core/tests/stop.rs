use samod_test_harness::{Network, RunningDocIds};

fn init_logging() {
    let _ = tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .try_init();
}

#[test]
fn test_stop_waits_for_save_tasks() {
    init_logging();
    let mut network = Network::new();
    let alice = network.create_samod("alice");

    let command_id = network.samod(&alice).start_create_document();
    network.samod(&alice).stop();
    let RunningDocIds { doc_id, .. } = network
        .samod(&alice)
        .check_create_document_result(command_id)
        .unwrap();

    // check that the document was saved to storage before shutdown by loading a
    // new peer pointing to the same storage
    let storage = network.samod(&alice).storage().clone();
    let alice_reloaded = network.create_samod_with_storage("alice_reloaded", storage);
    let _actor_id = network
        .samod(&alice_reloaded)
        .find_document(&doc_id)
        .expect("document should be found after reload");
}
