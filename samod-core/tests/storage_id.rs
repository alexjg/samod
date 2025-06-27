use samod_core::{StorageId, StorageKey};
use samod_test_harness::Network;

fn init_logging() {
    let _ = tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .try_init();
}

#[test]
fn test_storage_id_initialization() {
    init_logging();
    let mut network = Network::new();
    let alice = network.create_samod("Alice");

    // Process events to trigger storage ID initialization
    network.run_until_quiescent();

    // Storage ID should be available immediately after creation
    let storage_id = network.samod(&alice).storage_id();

    // Verify it's a valid UUID (non-nil)
    assert_ne!(storage_id, StorageId::from(uuid::Uuid::nil()));
}

#[test]
fn test_storage_id_persistence() {
    init_logging();
    let mut network1 = Network::new();
    let alice1 = network1.create_samod("Alice1");

    // Initialize storage ID
    network1.run_until_quiescent();
    let first_storage_id = network1.samod(&alice1).storage_id();

    // Extract the storage data from the first instance
    let storage_data = network1.samod(&alice1).storage().clone();

    // Create a second network instance with the same storage
    let mut network2 = Network::new();
    let alice2 = network2.create_samod_with_storage("Alice2", storage_data);

    // Process events to load existing storage ID
    network2.run_until_quiescent();

    let second_storage_id = network2.samod(&alice2).storage_id();

    // Both instances should have the same storage ID
    assert_eq!(first_storage_id, second_storage_id);
}

#[test]
fn test_storage_id_with_corrupted_data() {
    init_logging();
    let mut network = Network::new();

    // Put corrupted data in storage (wrong size)
    let alice_storage = vec![(StorageKey::storage_id_path(), vec![1, 2, 3])]
        .into_iter()
        .collect();
    // Initialize Alice with this corrupted storage
    let alice = network.create_samod_with_storage("Alice", alice_storage);

    // Process events to trigger initialization
    network.run_until_quiescent();

    // Should generate a new storage ID despite corrupted data
    let storage_id = network.samod(&alice).storage_id();

    // Verify new storage ID was written
    let stored_bytes = network
        .samod(&alice)
        .storage()
        .get(&StorageKey::storage_id_path())
        .cloned()
        .expect("Storage ID should be present");

    let stored_string = String::from_utf8(stored_bytes).expect("storage Id was not a valid string");

    let stored_id = StorageId::from(stored_string);
    assert_eq!(stored_id, storage_id);
}

#[test]
fn test_multiple_peers_same_storage() {
    init_logging();
    let mut network = Network::new();
    let alice = network.create_samod("alice");

    // Initialize Alice's storage ID first
    network.run_until_quiescent();
    let alice_storage_id = network.samod(&alice).storage_id();

    // Create Bob with Alice's storage (simulating shared storage like IndexedDB)
    let alice_storage = network.samod(&alice).storage().clone();
    let bob = network.create_samod_with_storage("bob", alice_storage);

    // Process Bob's events to load the shared storage ID
    network.run_until_quiescent();

    let bob_storage_id = network.samod(&bob).storage_id();

    // Both peers should have the same storage ID since they share storage
    assert_eq!(alice_storage_id, bob_storage_id);
}

#[test]
fn test_different_peers_different_storage() {
    init_logging();
    let mut network = Network::new();
    let alice = network.create_samod("Alice");
    let bob = network.create_samod("Bob");

    // Both peers process events independently (separate storage)
    network.run_until_quiescent();

    let alice_storage_id = network.samod(&alice).storage_id();

    let bob_storage_id = network.samod(&bob).storage_id();

    // Different peers with separate storage should have different storage IDs
    assert_ne!(alice_storage_id, bob_storage_id);
}
