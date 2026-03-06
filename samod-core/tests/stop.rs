use samod_core::actors::hub::HubEvent;
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

/// Regression test for https://github.com/alexjg/samod/issues/61
///
/// After the hub has stopped, a `ConnectionLost` event arriving (e.g. from
/// a background `drive_connection` task) must not panic. Before the fix the
/// hub contained `assert!(self.run_state != RunState::Stopped)` which would
/// fire in exactly this scenario.
#[test]
fn test_event_after_stop_does_not_panic() {
    init_logging();
    let mut network = Network::new();
    let alice = network.create_samod("alice");
    let bob = network.create_samod("bob");

    // Connect alice and bob
    let connected = network.connect(alice, bob);

    // Stop alice — this transitions the hub to Stopped
    network.samod(&alice).stop();

    // Simulate a late ConnectionLost event arriving after stop, as would
    // happen in the real async runtime when drive_connection notices the
    // transport has closed.
    let conn_id = connected.left;
    network
        .samod(&alice)
        .push_event(HubEvent::connection_lost(conn_id));
    network.samod(&alice).handle_events();
    // If we get here without panicking the fix works.
}

/// After the hub has stopped, a `Tick` event arriving (from the tick loop)
/// must not panic.
#[test]
fn test_tick_after_stop_does_not_panic() {
    init_logging();
    let mut network = Network::new();
    let alice = network.create_samod("alice");

    network.samod(&alice).stop();

    // Simulate a late tick arriving after stop
    network.samod(&alice).push_event(HubEvent::tick());
    network.samod(&alice).handle_events();
    // If we get here without panicking the fix works.
}
