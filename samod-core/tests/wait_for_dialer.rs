//! Tests for waiting on connecting dialers before marking documents as NotFound.
//!
//! When a document actor has no connected peers but there is a dialer actively
//! connecting (in NeedTransport or TransportPending state), the document should
//! not be marked NotFound — it should wait for the dialer to either connect
//! (at which point sync can proceed) or fail permanently (at which point we
//! give up).

use std::time::Duration;

use automerge::{ROOT, ReadDoc, transaction::Transactable};
use samod_core::{BackoffConfig, DialerConfig, DocumentId};
use samod_test_harness::{Network, RunningDocIds};

fn init_logging() {
    let _ = tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .try_init();
}

/// A dialer config with a long backoff so it stays in TransportPending and
/// doesn't auto-retry during the test.
fn non_retrying_dialer(url: &str) -> DialerConfig {
    DialerConfig {
        url: url::Url::parse(url).unwrap(),
        backoff: BackoffConfig {
            initial_delay: Duration::from_secs(999),
            max_delay: Duration::from_secs(999),
            max_retries: Some(0),
        },
    }
}

/// Helper macro: create a document on a peer, write some data, return the doc ID.
/// (Macro because SamodId is not publicly exported from the test harness.)
macro_rules! create_doc_with_data {
    ($network:expr, $peer:expr) => {{
        let RunningDocIds { doc_id, actor_id } = $network.samod(&$peer).create_document();
        $network
            .samod(&$peer)
            .with_document_by_actor(actor_id, |doc| {
                let mut tx = doc.transaction();
                tx.put(ROOT, "key", "value").unwrap();
                tx.commit();
            })
            .unwrap();
        doc_id
    }};
}

// =============================================================================
// Tests
// =============================================================================

/// When there are no dialers and no connections, find should resolve to
/// NotFound immediately (baseline behavior, unchanged).
#[test]
fn find_without_dialers_resolves_to_not_found() {
    init_logging();

    let mut network = Network::new();
    let bob = network.create_samod("Bob");

    let fake_doc_id = DocumentId::new(&mut rand::rng());
    let result = network.samod(&bob).find_document(&fake_doc_id);

    assert!(result.is_none(), "document should not be found");
}

/// When a dialer is in TransportPending state, find should NOT resolve to
/// NotFound — it should stay pending, waiting for the dialer to connect.
#[test]
fn find_with_connecting_dialer_stays_pending() {
    init_logging();

    let mut network = Network::new();
    let bob = network.create_samod("Bob");

    // Add a dialer — it enters TransportPending immediately
    let _dialer_id = network
        .samod(&bob)
        .add_dialer(non_retrying_dialer("wss://sync.example.com"));

    // Start a find for a document that doesn't exist locally
    let fake_doc_id = DocumentId::new(&mut rand::rng());
    let find_cmd = network.samod(&bob).begin_find_document(&fake_doc_id);

    // Process events
    network.run_until_quiescent();

    // The find should NOT have completed — the dialer is still connecting
    let result = network.samod(&bob).check_find_document_result(find_cmd);
    assert!(
        result.is_none(),
        "find should still be pending while dialer is connecting"
    );
}

#[test]
fn connected_with_long_handshake_does_not_mark_notfound() {
    // When a dialer connects in the hub, the status switches to
    // DialerStatus::Connected, but this doesn't mean the handshake is complete
    // which means that from the perspective of the document actor, the
    // connection is not yet ready. We want to make sure we wait until
    // the handshake is ready before returning NotFound
    init_logging();

    let mut network = Network::new();
    let bob = network.create_samod("Bob");

    // Add a dialer — it enters TransportPending immediately
    let dialer_id = network
        .samod(&bob)
        .add_dialer(non_retrying_dialer("wss://sync.example.com"));

    // Start a find for a document that doesn't exist locally
    let fake_doc_id = DocumentId::new(&mut rand::rng());
    let find_cmd = network.samod(&bob).begin_find_document(&fake_doc_id);

    // Process events
    network.run_until_quiescent();

    // Now mark the dialer as connected in the hub, but don't complete the handshake
    network.samod(&bob).create_dialer_connection(dialer_id);

    // The find should NOT have completed — the dialer is still connecting
    let result = network.samod(&bob).check_find_document_result(find_cmd);
    assert!(
        result.is_none(),
        "find should still be pending while dialer is connecting"
    );
}

/// When a dialer connects and the remote peer has the document, the find
/// should resolve to Ready. We connect the dialer before issuing find
/// because in the real runtime the handshake completes within the same
/// event loop iteration as `create_dialer_connection`, so the document
/// actor receives `NewConnection` before it can check dialer states.
#[test]
fn find_resolves_when_dialer_connects() {
    init_logging();

    let mut network = Network::new();
    let alice = network.create_samod("Alice");
    let bob = network.create_samod("Bob");

    // Alice creates a document with some data
    let doc_id = create_doc_with_data!(network, alice);

    // Bob adds a dialer and it connects to Alice
    let dialer_id = network
        .samod(&bob)
        .add_dialer(non_retrying_dialer("wss://alice.example.com"));
    network.connect_with_dialer(bob, dialer_id, alice);
    network.run_until_quiescent();

    // Now Bob finds the document — should sync from Alice and complete
    let result = network.samod(&bob).find_document(&doc_id);
    assert!(
        result.is_some(),
        "find should resolve to found after dialer connects"
    );

    // Verify the document content was synced
    let bob_ref = network.samod(&bob);
    let bob_doc = bob_ref.document(&doc_id).unwrap();
    let val = bob_doc
        .get(ROOT, "key")
        .unwrap()
        .map(|(v, _)| v.to_string())
        .unwrap_or_default();
    assert_eq!(val, r#""value""#);
}

/// When a dialer fails permanently (max retries exhausted), the find should
/// resolve to NotFound.
#[test]
fn find_resolves_to_not_found_when_dialer_fails() {
    init_logging();

    let mut network = Network::new();
    let bob = network.create_samod("Bob");

    // Add a dialer with max_retries=0 so it fails permanently on first failure
    let dialer_id = network
        .samod(&bob)
        .add_dialer(non_retrying_dialer("wss://unreachable.example.com"));

    // Start finding a document
    let fake_doc_id = DocumentId::new(&mut rand::rng());
    let find_cmd = network.samod(&bob).begin_find_document(&fake_doc_id);
    network.run_until_quiescent();

    // Find should still be pending
    let result = network.samod(&bob).check_find_document_result(find_cmd);
    assert!(
        result.is_none(),
        "find should be pending while dialer is connecting"
    );

    // Dialer fails permanently
    network
        .samod(&bob)
        .dial_failed(dialer_id, "connection refused".to_string());
    network.run_until_quiescent();

    // Now the find should complete as NotFound
    let result = network.samod(&bob).check_find_document_result(find_cmd);
    assert!(
        matches!(result, Some(None)),
        "find should resolve to not found after dialer fails: got {result:?}"
    );
}

/// A dialer in WaitingToRetry state should NOT block NotFound. WaitingToRetry
/// means the dialer will try again later, but we don't hold up the find for it.
#[test]
fn waiting_to_retry_does_not_block_not_found() {
    init_logging();

    let mut network = Network::new();
    let bob = network.create_samod("Bob");

    // Add a dialer that will retry (not capped) with a long delay
    let dialer_id = network.samod(&bob).add_dialer(DialerConfig {
        url: url::Url::parse("wss://flaky.example.com").unwrap(),
        backoff: BackoffConfig {
            initial_delay: Duration::from_secs(999),
            max_delay: Duration::from_secs(999),
            max_retries: None, // will retry indefinitely
        },
    });

    // Start finding a document — pending because dialer is connecting
    let fake_doc_id = DocumentId::new(&mut rand::rng());
    let find_cmd = network.samod(&bob).begin_find_document(&fake_doc_id);
    network.run_until_quiescent();

    let result = network.samod(&bob).check_find_document_result(find_cmd);
    assert!(
        result.is_none(),
        "find should be pending while dialer is connecting"
    );

    // Dialer fails but will retry — transitions to WaitingToRetry
    network
        .samod(&bob)
        .dial_failed(dialer_id, "connection refused".to_string());
    network.run_until_quiescent();

    // WaitingToRetry should NOT block NotFound
    let result = network.samod(&bob).check_find_document_result(find_cmd);
    assert!(
        matches!(result, Some(None)),
        "find should resolve to not found when dialer is in WaitingToRetry: got {result:?}"
    );
}

/// Removing a dialer while a find is waiting on it should cause the find to
/// resolve to NotFound (assuming no other connections or connecting dialers).
#[test]
fn removing_dialer_while_waiting_triggers_not_found() {
    init_logging();

    let mut network = Network::new();
    let bob = network.create_samod("Bob");

    // Add a dialer
    let dialer_id = network
        .samod(&bob)
        .add_dialer(non_retrying_dialer("wss://sync.example.com"));

    // Start finding a document
    let fake_doc_id = DocumentId::new(&mut rand::rng());
    let find_cmd = network.samod(&bob).begin_find_document(&fake_doc_id);
    network.run_until_quiescent();

    // Find should be pending
    let result = network.samod(&bob).check_find_document_result(find_cmd);
    assert!(
        result.is_none(),
        "find should be pending while dialer exists"
    );

    // Remove the dialer entirely
    network.samod(&bob).remove_dialer(dialer_id);
    network.run_until_quiescent();

    // Find should now complete as NotFound
    let result = network.samod(&bob).check_find_document_result(find_cmd);
    assert!(
        matches!(result, Some(None)),
        "find should resolve to not found after dialer is removed: got {result:?}"
    );
}

/// With multiple dialers, the find should stay pending as long as ANY dialer
/// is still connecting, even if others have failed.
#[test]
fn multiple_dialers_stays_pending_while_any_connecting() {
    init_logging();

    let mut network = Network::new();
    let bob = network.create_samod("Bob");

    // Add two dialers
    let dialer1 = network
        .samod(&bob)
        .add_dialer(non_retrying_dialer("wss://server1.example.com"));
    let _dialer2 = network
        .samod(&bob)
        .add_dialer(non_retrying_dialer("wss://server2.example.com"));

    // Start finding a document
    let fake_doc_id = DocumentId::new(&mut rand::rng());
    let find_cmd = network.samod(&bob).begin_find_document(&fake_doc_id);
    network.run_until_quiescent();

    // Find should be pending
    let result = network.samod(&bob).check_find_document_result(find_cmd);
    assert!(
        result.is_none(),
        "find should be pending with two connecting dialers"
    );

    // First dialer fails permanently
    network
        .samod(&bob)
        .dial_failed(dialer1, "connection refused".to_string());
    network.run_until_quiescent();

    // Find should STILL be pending — dialer2 is still connecting
    let result = network.samod(&bob).check_find_document_result(find_cmd);
    assert!(
        result.is_none(),
        "find should still be pending while second dialer is connecting: got {result:?}"
    );
}

/// With multiple dialers, the find should resolve to NotFound only when ALL
/// connecting dialers have failed.
#[test]
fn multiple_dialers_not_found_when_all_fail() {
    init_logging();

    let mut network = Network::new();
    let bob = network.create_samod("Bob");

    // Add two dialers
    let dialer1 = network
        .samod(&bob)
        .add_dialer(non_retrying_dialer("wss://server1.example.com"));
    let dialer2 = network
        .samod(&bob)
        .add_dialer(non_retrying_dialer("wss://server2.example.com"));

    // Start finding a document
    let fake_doc_id = DocumentId::new(&mut rand::rng());
    let find_cmd = network.samod(&bob).begin_find_document(&fake_doc_id);
    network.run_until_quiescent();

    // Both dialers fail
    network
        .samod(&bob)
        .dial_failed(dialer1, "connection refused".to_string());
    network
        .samod(&bob)
        .dial_failed(dialer2, "connection refused".to_string());
    network.run_until_quiescent();

    // Now find should resolve to NotFound
    let result = network.samod(&bob).check_find_document_result(find_cmd);
    assert!(
        matches!(result, Some(None)),
        "find should resolve to not found when all dialers have failed: got {result:?}"
    );
}
