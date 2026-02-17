#![cfg(feature = "tokio")]

use std::{
    pin::Pin,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};

use automerge::{Automerge, ReadDoc};
use futures::{Sink, SinkExt, Stream, StreamExt};
use samod::{
    AcceptorEvent, AcceptorHandle, BackoffConfig, Dialer, DialerEvent, PeerId, Repo, Transport,
};
use tokio_stream::wrappers::ReceiverStream;
use tokio_util::sync::PollSender;
use url::Url;

fn init_logging() {
    let _ = tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .try_init();
}

// =============================================================================
// In-memory transport helpers
// =============================================================================

#[derive(Debug)]
struct MemError(String);

impl std::fmt::Display for MemError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl std::error::Error for MemError {}

/// One side of an in-memory transport.
struct MemTransportSide {
    send: Box<dyn Send + Unpin + Sink<Vec<u8>, Error = MemError>>,
    recv: Box<dyn Send + Unpin + Stream<Item = Result<Vec<u8>, MemError>>>,
}

/// Create a pair of in-memory transport sides.
fn mem_transport_pair() -> (MemTransportSide, MemTransportSide) {
    let (a_tx, b_rx) = tokio::sync::mpsc::channel::<Vec<u8>>(16);
    let (b_tx, a_rx) = tokio::sync::mpsc::channel::<Vec<u8>>(16);

    let a = MemTransportSide {
        send: Box::new(
            PollSender::new(a_tx).sink_map_err(|e| MemError(format!("send error: {e:?}"))),
        ),
        recv: Box::new(ReceiverStream::new(a_rx).map(Ok)),
    };

    let b = MemTransportSide {
        send: Box::new(
            PollSender::new(b_tx).sink_map_err(|e| MemError(format!("send error: {e:?}"))),
        ),
        recv: Box::new(ReceiverStream::new(b_rx).map(Ok)),
    };

    (a, b)
}

// =============================================================================
// Mock Dialer: connects via in-memory channels to an acceptor handle
// =============================================================================

/// A mock dialer that connects to a target `AcceptorHandle` via in-memory channels.
///
/// Each call to `connect()` creates a fresh pair of in-memory channels,
/// feeds one side to the acceptor, and returns the other as a `Transport`.
struct MockDialer {
    url: Url,
    acceptor: AcceptorHandle,
    connect_count: AtomicUsize,
}

impl MockDialer {
    fn new(url: Url, acceptor: AcceptorHandle) -> Self {
        Self {
            url,
            acceptor,
            connect_count: AtomicUsize::new(0),
        }
    }
}

impl Dialer for MockDialer {
    fn url(&self) -> Url {
        self.url.clone()
    }

    fn connect(
        &self,
    ) -> Pin<
        Box<
            dyn std::future::Future<
                    Output = Result<Transport, Box<dyn std::error::Error + Send + Sync + 'static>>,
                > + Send,
        >,
    > {
        self.connect_count.fetch_add(1, Ordering::SeqCst);

        let (dialer_side, acceptor_side) = mem_transport_pair();
        let acceptor = self.acceptor.clone();

        Box::pin(async move {
            // Feed the acceptor side to the acceptor handle
            acceptor
                .accept(Transport::new(acceptor_side.recv, acceptor_side.send))
                .map_err(|e| Box::new(e) as Box<dyn std::error::Error + Send + Sync + 'static>)?;

            Ok(Transport::new(dialer_side.recv, dialer_side.send))
        })
    }
}

/// A mock dialer that always fails.
struct FailingDialer {
    url: Url,
    fail_count: AtomicUsize,
}

impl FailingDialer {
    fn new(url: Url) -> Self {
        Self {
            url,
            fail_count: AtomicUsize::new(0),
        }
    }
}

impl Dialer for FailingDialer {
    fn url(&self) -> Url {
        self.url.clone()
    }

    fn connect(
        &self,
    ) -> Pin<
        Box<
            dyn std::future::Future<
                    Output = Result<Transport, Box<dyn std::error::Error + Send + Sync + 'static>>,
                > + Send,
        >,
    > {
        self.fail_count.fetch_add(1, Ordering::SeqCst);
        Box::pin(async { Err("connection refused".into()) })
    }
}

/// A mock dialer that fails N times then succeeds by connecting to an acceptor.
struct FailThenSucceedDialer {
    url: Url,
    acceptor: AcceptorHandle,
    fail_times: usize,
    attempt: AtomicUsize,
}

impl FailThenSucceedDialer {
    fn new(url: Url, acceptor: AcceptorHandle, fail_times: usize) -> Self {
        Self {
            url,
            acceptor,
            fail_times,
            attempt: AtomicUsize::new(0),
        }
    }
}

impl Dialer for FailThenSucceedDialer {
    fn url(&self) -> Url {
        self.url.clone()
    }

    fn connect(
        &self,
    ) -> Pin<
        Box<
            dyn std::future::Future<
                    Output = Result<Transport, Box<dyn std::error::Error + Send + Sync + 'static>>,
                > + Send,
        >,
    > {
        let attempt = self.attempt.fetch_add(1, Ordering::SeqCst);
        if attempt < self.fail_times {
            Box::pin(async move { Err(format!("connection refused (attempt {attempt})").into()) })
        } else {
            let (dialer_side, acceptor_side) = mem_transport_pair();
            let acceptor = self.acceptor.clone();
            Box::pin(async move {
                acceptor
                    .accept(Transport::new(acceptor_side.recv, acceptor_side.send))
                    .map_err(|e| {
                        Box::new(e) as Box<dyn std::error::Error + Send + Sync + 'static>
                    })?;
                Ok(Transport::new(dialer_side.recv, dialer_side.send))
            })
        }
    }
}

// =============================================================================
// Acceptor tests
// =============================================================================

#[tokio::test]
async fn acceptor_returns_handle_with_valid_id() {
    init_logging();
    let repo = Repo::build_tokio()
        .with_peer_id(PeerId::from("server"))
        .load()
        .await;

    let url = Url::parse("ws://0.0.0.0:8080").unwrap();
    let handle = repo.make_acceptor(url).unwrap();

    // Should have a valid connector ID
    assert_eq!(handle.connection_count(), 0);

    repo.stop().await;
}

#[tokio::test]
async fn acceptor_accept_wires_connection() {
    init_logging();
    let server = Repo::build_tokio()
        .with_peer_id(PeerId::from("server"))
        .load()
        .await;

    let client = Repo::build_tokio()
        .with_peer_id(PeerId::from("client"))
        .load()
        .await;

    let url = Url::parse("ws://0.0.0.0:8080").unwrap();
    let acceptor = server.make_acceptor(url.clone()).unwrap();
    let mut events = acceptor.events();

    // Client dials the server via MockDialer
    let dialer = MockDialer::new(url, acceptor.clone());
    let handle = client
        .dial(BackoffConfig::default(), Arc::new(dialer))
        .unwrap();

    // Wait for handshake on client side
    let peer_info = tokio::time::timeout(Duration::from_secs(5), handle.established())
        .await
        .expect("handshake timed out")
        .expect("handshake failed");

    assert_eq!(peer_info.peer_id, PeerId::from("server"));

    // Check acceptor event
    let event = tokio::time::timeout(Duration::from_secs(5), events.next())
        .await
        .expect("event timed out")
        .expect("event stream ended");

    match event {
        AcceptorEvent::ClientConnected { peer_info, .. } => {
            assert_eq!(peer_info.peer_id, PeerId::from("client"));
        }
        other => panic!("expected ClientConnected, got {:?}", other),
    }

    assert_eq!(acceptor.connection_count(), 1);

    handle.close();
    server.stop().await;
    client.stop().await;
}

#[tokio::test]
async fn acceptor_multiple_clients() {
    init_logging();
    let server = Repo::build_tokio()
        .with_peer_id(PeerId::from("server"))
        .load()
        .await;

    let url = Url::parse("ws://0.0.0.0:8080").unwrap();
    let acceptor = server.make_acceptor(url.clone()).unwrap();

    // Accept two clients via MockDialer
    for i in 0..2 {
        let client = Repo::build_tokio()
            .with_peer_id(PeerId::from(format!("client-{i}")))
            .load()
            .await;

        let dialer = MockDialer::new(url.clone(), acceptor.clone());
        let handle = client
            .dial(BackoffConfig::default(), Arc::new(dialer))
            .unwrap();

        tokio::time::timeout(Duration::from_secs(5), handle.established())
            .await
            .expect("handshake timed out")
            .expect("handshake failed");
    }

    assert_eq!(acceptor.connection_count(), 2);

    server.stop().await;
}

#[tokio::test]
async fn acceptor_close_disconnects_all() {
    init_logging();
    let server = Repo::build_tokio()
        .with_peer_id(PeerId::from("server"))
        .load()
        .await;

    let url = Url::parse("ws://0.0.0.0:8080").unwrap();
    let acceptor = server.make_acceptor(url.clone()).unwrap();

    // Accept one client via MockDialer
    let client = Repo::build_tokio()
        .with_peer_id(PeerId::from("client"))
        .load()
        .await;

    let dialer = MockDialer::new(url, acceptor.clone());
    let handle = client
        .dial(BackoffConfig::default(), Arc::new(dialer))
        .unwrap();

    tokio::time::timeout(Duration::from_secs(5), handle.established())
        .await
        .expect("handshake timed out")
        .expect("handshake failed");

    assert_eq!(acceptor.connection_count(), 1);

    // Close the acceptor — should not panic
    acceptor.close();

    // Server should still stop cleanly after closing the acceptor
    tokio::time::timeout(Duration::from_secs(5), server.stop())
        .await
        .expect("server.stop() timed out");
    client.stop().await;
}

// =============================================================================
// Acceptor URL reuse tests
// =============================================================================

#[tokio::test]
async fn acceptor_same_url_returns_same_handle() {
    init_logging();
    let server = Repo::build_tokio()
        .with_peer_id(PeerId::from("server"))
        .load()
        .await;

    let url = Url::parse("ws://0.0.0.0:8080").unwrap();

    let handle1 = server.make_acceptor(url.clone()).unwrap();
    let handle2 = server.make_acceptor(url).unwrap();

    // Both should use the same listener (same URL)
    assert_eq!(handle1.id(), handle2.id());

    server.stop().await;
}

#[tokio::test]
async fn acceptor_different_urls_create_separate_listeners() {
    init_logging();
    let server = Repo::build_tokio()
        .with_peer_id(PeerId::from("server"))
        .load()
        .await;

    let url1 = Url::parse("ws://0.0.0.0:8080").unwrap();
    let url2 = Url::parse("ws://0.0.0.0:9090").unwrap();

    let handle1 = server.make_acceptor(url1).unwrap();
    let handle2 = server.make_acceptor(url2).unwrap();

    // Different URLs should create different listeners
    assert_ne!(handle1.id(), handle2.id());

    server.stop().await;
}

// =============================================================================
// Dialer tests
// =============================================================================

#[tokio::test]
async fn dial_returns_handle() {
    init_logging();
    let alice = Repo::build_tokio()
        .with_peer_id(PeerId::from("alice"))
        .load()
        .await;

    let bob = Repo::build_tokio()
        .with_peer_id(PeerId::from("bob"))
        .load()
        .await;

    let url = Url::parse("ws://localhost:8080").unwrap();
    let acceptor = bob.make_acceptor(url.clone()).unwrap();
    let dialer = MockDialer::new(url, acceptor);

    let handle = alice
        .dial(BackoffConfig::default(), Arc::new(dialer))
        .unwrap();

    // Should not be connected yet (async handshake hasn't completed)
    // Note: it might connect very fast in tests, so we just check the handle exists
    let _id = handle.id();

    // Clean up
    handle.close();
    alice.stop().await;
    bob.stop().await;
}

#[tokio::test]
async fn dial_established_resolves_on_connect() {
    init_logging();
    let alice = Repo::build_tokio()
        .with_peer_id(PeerId::from("alice"))
        .load()
        .await;

    let bob = Repo::build_tokio()
        .with_peer_id(PeerId::from("bob"))
        .load()
        .await;

    let url = Url::parse("ws://localhost:8080").unwrap();
    let acceptor = bob.make_acceptor(url.clone()).unwrap();
    let dialer = MockDialer::new(url, acceptor);

    let handle = alice
        .dial(BackoffConfig::default(), Arc::new(dialer))
        .unwrap();

    let peer_info = tokio::time::timeout(Duration::from_secs(5), handle.established())
        .await
        .expect("established timed out")
        .expect("established failed");

    assert_eq!(peer_info.peer_id, PeerId::from("bob"));
    assert!(handle.is_connected());
    assert_eq!(
        handle.peer_info().map(|p| p.peer_id),
        Some(PeerId::from("bob"))
    );

    handle.close();
    alice.stop().await;
    bob.stop().await;
}

#[tokio::test]
async fn dial_events_stream_connected() {
    init_logging();
    let alice = Repo::build_tokio()
        .with_peer_id(PeerId::from("alice"))
        .load()
        .await;

    let bob = Repo::build_tokio()
        .with_peer_id(PeerId::from("bob"))
        .load()
        .await;

    let url = Url::parse("ws://localhost:8080").unwrap();
    let acceptor = bob.make_acceptor(url.clone()).unwrap();
    let dialer = MockDialer::new(url, acceptor);

    let handle = alice
        .dial(BackoffConfig::default(), Arc::new(dialer))
        .unwrap();

    let mut events = handle.events();

    let event = tokio::time::timeout(Duration::from_secs(5), events.next())
        .await
        .expect("event timed out")
        .expect("event stream ended");

    match event {
        DialerEvent::Connected { peer_info } => {
            assert_eq!(peer_info.peer_id, PeerId::from("bob"));
        }
        other => panic!("expected Connected, got {:?}", other),
    }

    handle.close();
    alice.stop().await;
    bob.stop().await;
}

#[tokio::test]
async fn dial_and_accept_sync_documents() {
    init_logging();
    let alice = Repo::build_tokio()
        .with_peer_id(PeerId::from("alice"))
        .load()
        .await;

    let bob = Repo::build_tokio()
        .with_peer_id(PeerId::from("bob"))
        .load()
        .await;

    let url = Url::parse("ws://localhost:8080").unwrap();
    let acceptor = bob.make_acceptor(url.clone()).unwrap();
    let acceptor_for_events = acceptor.clone();
    let mut acceptor_events = acceptor_for_events.events();
    let dialer = MockDialer::new(url, acceptor);

    let handle = alice
        .dial(BackoffConfig::default(), Arc::new(dialer))
        .unwrap();

    // Wait for connection on Alice's side
    tokio::time::timeout(Duration::from_secs(5), handle.established())
        .await
        .expect("established timed out")
        .expect("established failed");

    // Get Bob's connection ID from the acceptor event
    let bob_conn_id = match tokio::time::timeout(Duration::from_secs(5), acceptor_events.next())
        .await
        .expect("acceptor event timed out")
        .expect("acceptor event stream ended")
    {
        AcceptorEvent::ClientConnected { connection_id, .. } => connection_id,
        other => panic!("expected ClientConnected, got {:?}", other),
    };

    // Create a document on Alice
    let alice_doc = alice.create(Automerge::new()).await.unwrap();
    alice_doc.with_document(|am| {
        use automerge::{AutomergeError, ROOT};
        am.transact::<_, _, AutomergeError>(|tx| {
            use automerge::transaction::Transactable;
            tx.put(ROOT, "hello", "world")?;
            Ok(())
        })
        .unwrap();
    });

    // Alice waits until her changes are sent to Bob
    let alice_conn_id = handle.connection_id().expect("should be connected");
    tokio::time::timeout(
        Duration::from_secs(5),
        alice_doc.they_have_our_changes(alice_conn_id),
    )
    .await
    .expect("alice -> bob sync timed out");

    // Bob should be able to find the document
    let bob_doc = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if let Some(doc) = bob.find(alice_doc.document_id().clone()).await.unwrap() {
                return doc;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    })
    .await
    .expect("bob never found alice's document");

    // Wait for Bob to have Alice's changes
    tokio::time::timeout(
        Duration::from_secs(5),
        bob_doc.we_have_their_changes(bob_conn_id),
    )
    .await
    .expect("bob content sync timed out");

    bob_doc.with_document(|am| {
        let value = am
            .get(automerge::ROOT, "hello")
            .unwrap()
            .map(|(v, _)| v.into_string().unwrap());
        assert_eq!(value.as_deref(), Some("world"));
    });

    handle.close();
    alice.stop().await;
    bob.stop().await;
}

#[tokio::test]
async fn dial_bidirectional_sync() {
    init_logging();
    let alice = Repo::build_tokio()
        .with_peer_id(PeerId::from("alice"))
        .load()
        .await;

    let bob = Repo::build_tokio()
        .with_peer_id(PeerId::from("bob"))
        .load()
        .await;

    let url = Url::parse("ws://localhost:8080").unwrap();
    let acceptor = bob.make_acceptor(url.clone()).unwrap();
    let dialer = MockDialer::new(url, acceptor);

    let handle = alice
        .dial(BackoffConfig::default(), Arc::new(dialer))
        .unwrap();

    tokio::time::timeout(Duration::from_secs(5), handle.established())
        .await
        .expect("established timed out")
        .expect("established failed");

    // Create doc on Alice
    let alice_doc = alice.create(Automerge::new()).await.unwrap();
    alice_doc.with_document(|am| {
        use automerge::{AutomergeError, ROOT};
        am.transact::<_, _, AutomergeError>(|tx| {
            use automerge::transaction::Transactable;
            tx.put(ROOT, "from", "alice")?;
            Ok(())
        })
        .unwrap();
    });

    // Create doc on Bob
    let bob_doc = bob.create(Automerge::new()).await.unwrap();
    bob_doc.with_document(|am| {
        use automerge::{AutomergeError, ROOT};
        am.transact::<_, _, AutomergeError>(|tx| {
            use automerge::transaction::Transactable;
            tx.put(ROOT, "from", "bob")?;
            Ok(())
        })
        .unwrap();
    });

    // Wait for Alice to find Bob's doc
    let _alice_finds_bob = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if let Some(doc) = alice.find(bob_doc.document_id().clone()).await.unwrap() {
                return doc;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    })
    .await
    .expect("alice never found bob's document");

    // Wait for Bob to find Alice's doc
    let _bob_finds_alice = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if let Some(doc) = bob.find(alice_doc.document_id().clone()).await.unwrap() {
                return doc;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    })
    .await
    .expect("bob never found alice's document");

    handle.close();
    alice.stop().await;
    bob.stop().await;
}

// =============================================================================
// Failure / retry tests
// =============================================================================

#[tokio::test]
async fn dial_max_retries_emits_failure() {
    init_logging();
    let alice = Repo::build_tokio()
        .with_peer_id(PeerId::from("alice"))
        .load()
        .await;

    let url = Url::parse("ws://localhost:9999").unwrap();
    let dialer = Arc::new(FailingDialer::new(url));

    let handle = alice
        .dial(
            BackoffConfig {
                initial_delay: Duration::from_millis(10),
                max_delay: Duration::from_millis(50),
                max_retries: Some(2),
            },
            dialer.clone(),
        )
        .unwrap();

    // established() should return Err when max retries are reached
    let result = tokio::time::timeout(Duration::from_secs(10), handle.established())
        .await
        .expect("established timed out");

    assert!(result.is_err());
    assert!(!handle.is_connected());

    alice.stop().await;
}

#[tokio::test]
async fn dial_events_max_retries_reached() {
    init_logging();
    let alice = Repo::build_tokio()
        .with_peer_id(PeerId::from("alice"))
        .load()
        .await;

    let url = Url::parse("ws://localhost:9999").unwrap();
    let dialer = Arc::new(FailingDialer::new(url));

    let handle = alice
        .dial(
            BackoffConfig {
                initial_delay: Duration::from_millis(10),
                max_delay: Duration::from_millis(50),
                max_retries: Some(1),
            },
            dialer.clone(),
        )
        .unwrap();

    let mut events = handle.events();

    // Collect events until MaxRetriesReached
    let found_max_retries = tokio::time::timeout(Duration::from_secs(10), async {
        while let Some(event) = events.next().await {
            if matches!(event, DialerEvent::MaxRetriesReached) {
                return true;
            }
        }
        false
    })
    .await
    .expect("event stream timed out");

    assert!(
        found_max_retries,
        "should have received MaxRetriesReached event"
    );

    alice.stop().await;
}

#[tokio::test]
async fn dial_recovers_after_initial_failures() {
    init_logging();
    let alice = Repo::build_tokio()
        .with_peer_id(PeerId::from("alice"))
        .load()
        .await;

    let bob = Repo::build_tokio()
        .with_peer_id(PeerId::from("bob"))
        .load()
        .await;

    let url = Url::parse("ws://localhost:8080").unwrap();
    let acceptor = bob.make_acceptor(url.clone()).unwrap();

    // Fail 2 times, then succeed
    let dialer = FailThenSucceedDialer::new(url, acceptor, 2);

    let handle = alice
        .dial(
            BackoffConfig {
                initial_delay: Duration::from_millis(10),
                max_delay: Duration::from_millis(100),
                max_retries: None, // unlimited retries
            },
            Arc::new(dialer),
        )
        .unwrap();

    let peer_info = tokio::time::timeout(Duration::from_secs(10), handle.established())
        .await
        .expect("established timed out")
        .expect("established failed — should have recovered");

    assert_eq!(peer_info.peer_id, PeerId::from("bob"));

    handle.close();
    alice.stop().await;
    bob.stop().await;
}

// =============================================================================
// Lifecycle / cleanup tests
// =============================================================================

#[tokio::test]
async fn remove_connector_by_handle() {
    init_logging();
    let alice = Repo::build_tokio()
        .with_peer_id(PeerId::from("alice"))
        .load()
        .await;

    let bob = Repo::build_tokio()
        .with_peer_id(PeerId::from("bob"))
        .load()
        .await;

    let url = Url::parse("ws://localhost:8080").unwrap();
    let acceptor = bob.make_acceptor(url.clone()).unwrap();
    let dialer = MockDialer::new(url, acceptor);

    let handle = alice
        .dial(BackoffConfig::default(), Arc::new(dialer))
        .unwrap();

    tokio::time::timeout(Duration::from_secs(5), handle.established())
        .await
        .expect("established timed out")
        .expect("established failed");

    assert!(handle.is_connected());

    // Close the dialer — this removes the connector internally
    handle.close();

    // Repo should still stop cleanly after closing a connector
    tokio::time::timeout(Duration::from_secs(5), alice.stop())
        .await
        .expect("alice.stop() timed out");
    bob.stop().await;
}

#[tokio::test]
async fn stop_repo_with_active_connectors() {
    init_logging();
    let alice = Repo::build_tokio()
        .with_peer_id(PeerId::from("alice"))
        .load()
        .await;

    let bob = Repo::build_tokio()
        .with_peer_id(PeerId::from("bob"))
        .load()
        .await;

    let url = Url::parse("ws://localhost:8080").unwrap();
    let acceptor = bob.make_acceptor(url.clone()).unwrap();
    let dialer = MockDialer::new(url, acceptor.clone());

    let handle = alice
        .dial(BackoffConfig::default(), Arc::new(dialer))
        .unwrap();

    tokio::time::timeout(Duration::from_secs(5), handle.established())
        .await
        .expect("established timed out")
        .expect("established failed");

    // Stop both repos — should not panic
    tokio::time::timeout(Duration::from_secs(5), alice.stop())
        .await
        .expect("alice.stop() timed out");

    tokio::time::timeout(Duration::from_secs(5), bob.stop())
        .await
        .expect("bob.stop() timed out");
}

#[tokio::test]
async fn dial_on_stopped_repo_returns_error() {
    init_logging();
    let alice = Repo::build_tokio()
        .with_peer_id(PeerId::from("alice"))
        .load()
        .await;

    let bob = Repo::build_tokio()
        .with_peer_id(PeerId::from("bob"))
        .load()
        .await;

    let url = Url::parse("ws://localhost:8080").unwrap();
    let acceptor = bob.make_acceptor(url.clone()).unwrap();

    alice.stop().await;

    let dialer = MockDialer::new(url, acceptor);
    let result = alice.dial(BackoffConfig::default(), Arc::new(dialer));
    assert!(result.is_err(), "dial on stopped repo should return Err");

    bob.stop().await;
}

// =============================================================================
// Multi-peer sync via server relay
// =============================================================================

#[tokio::test]
async fn multi_peer_sync_via_server() {
    init_logging();
    let server = Repo::build_tokio()
        .with_peer_id(PeerId::from("server"))
        .load()
        .await;

    let client_a = Repo::build_tokio()
        .with_peer_id(PeerId::from("client-a"))
        .load()
        .await;

    let client_b = Repo::build_tokio()
        .with_peer_id(PeerId::from("client-b"))
        .load()
        .await;

    let url = Url::parse("ws://localhost:8080").unwrap();
    let acceptor = server.make_acceptor(url.clone()).unwrap();

    // Both clients dial the server
    let dialer_a = MockDialer::new(url.clone(), acceptor.clone());
    let handle_a = client_a
        .dial(BackoffConfig::default(), Arc::new(dialer_a))
        .unwrap();

    let dialer_b = MockDialer::new(url, acceptor);
    let handle_b = client_b
        .dial(BackoffConfig::default(), Arc::new(dialer_b))
        .unwrap();

    // Wait for both to connect
    tokio::time::timeout(Duration::from_secs(5), handle_a.established())
        .await
        .expect("client_a established timed out")
        .expect("client_a established failed");

    tokio::time::timeout(Duration::from_secs(5), handle_b.established())
        .await
        .expect("client_b established timed out")
        .expect("client_b established failed");

    // Client A creates a document
    let doc_a = client_a.create(Automerge::new()).await.unwrap();
    doc_a.with_document(|am| {
        use automerge::{AutomergeError, ROOT};
        am.transact::<_, _, AutomergeError>(|tx| {
            use automerge::transaction::Transactable;
            tx.put(ROOT, "author", "client-a")?;
            Ok(())
        })
        .unwrap();
    });

    // Client B should eventually find it (via server relay)
    let _b_finds_doc = tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            if let Some(doc) = client_b.find(doc_a.document_id().clone()).await.unwrap() {
                return doc;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    })
    .await
    .expect("client_b never found client_a's document");

    // Finding the document proves it was relayed through the server.
    // Content sync timing is tested in dial_and_accept_sync_documents.

    handle_a.close();
    handle_b.close();
    server.stop().await;
    client_a.stop().await;
    client_b.stop().await;
}
