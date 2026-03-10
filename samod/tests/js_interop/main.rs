use std::time::Duration;

use automerge::{Automerge, ReadDoc, transaction::Transactable};
use futures::StreamExt;
use samod::{AcceptorHandle, BackoffConfig, PeerId, Repo};

mod js_wrapper;
use js_wrapper::JsWrapper;
use tokio::net::TcpListener;
use url::Url;

fn init_logging() {
    let _ = tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .try_init();
}

#[tokio::test]
async fn sync_rust_clients_via_js_server() {
    init_logging();
    let js = JsWrapper::create().await.unwrap();
    let js_server = js.start_server().await.unwrap();
    let port = js_server.port;

    let repo1 = samod_connected_to_js_server(port, Some("repo1".to_string())).await;

    let doc_handle_repo1 = repo1.create(Automerge::new()).await.unwrap();
    doc_handle_repo1
        .with_document(|doc| {
            doc.transact(|tx| {
                tx.put(automerge::ROOT, "key", "value")?;
                Ok::<_, automerge::AutomergeError>(())
            })
        })
        .unwrap();

    let repo2 = samod_connected_to_js_server(port, Some("repo2".to_string())).await;

    tokio::time::sleep(Duration::from_millis(1000)).await;

    let doc_handle_repo2 = repo2
        .find(doc_handle_repo1.document_id().clone())
        .await
        .unwrap()
        .unwrap();
    doc_handle_repo2.with_document(|doc| {
        assert_eq!(
            doc.get::<_, &str>(automerge::ROOT, "key")
                .unwrap()
                .unwrap()
                .0
                .into_string()
                .unwrap()
                .as_str(),
            "value"
        );
    });
}

#[tokio::test]
async fn two_js_clients_can_sync_through_rust_server() {
    init_logging();
    let server = start_rust_server().await;
    let js = JsWrapper::create().await.unwrap();
    let (doc_id, heads, _child1) = js.create_doc(server.port).await.unwrap();

    let fetched_heads = js.fetch_doc(server.port, doc_id).await.unwrap();

    assert_eq!(heads, fetched_heads);
}

#[tokio::test]
async fn send_ephemeral_messages_from_rust_clients_via_js_server() {
    let js = JsWrapper::create().await.unwrap();
    let js_server = js.start_server().await.unwrap();
    let port = js_server.port;

    let repo1 = samod_connected_to_js_server(port, Some("repo1".to_string())).await;

    let doc_handle_repo1 = repo1.create(Automerge::new()).await.unwrap();

    let repo2 = samod_connected_to_js_server(port, Some("repo2".to_string())).await;

    tokio::time::sleep(Duration::from_millis(1000)).await;

    let doc_handle_repo2 = repo2
        .find(doc_handle_repo1.document_id().clone())
        .await
        .unwrap()
        .unwrap();

    let mut ephemera = doc_handle_repo2.ephemera().boxed();

    // A cbor array of two integers
    let msg: Vec<u8> = vec![0x82, 0x01, 0x02];

    doc_handle_repo1.broadcast(msg.clone());

    let received = tokio::time::timeout(Duration::from_millis(1000), ephemera.next())
        .await
        .expect("timed out waiting for ephemeral message")
        .expect("no ephemeral message received");

    assert_eq!(received, msg);
}

#[tokio::test]
async fn two_js_clients_can_send_ephemera_through_rust_server() {
    let js = JsWrapper::create().await.unwrap();
    let server = start_rust_server().await;

    let (doc_id, _heads, _child1) = js.create_doc(server.port).await.unwrap();

    let mut listening = js
        .receive_ephemera(server.port, doc_id.clone())
        .await
        .unwrap();

    tokio::time::timeout(
        Duration::from_millis(2000),
        js.send_ephemeral_message(server.port, doc_id, "hello"),
    )
    .await
    .expect("timed out sending ephemeral message")
    .expect("error sending ephemeral message");

    let msg = tokio::time::timeout(Duration::from_millis(1000), listening.next())
        .await
        .expect("timed out waiting for ephemeral message")
        .expect("no ephemeral message received")
        .expect("error reading ephemeral message");

    assert_eq!(msg, "hello");
}

/// Test that a JS client which uses remote heads subscriptions (sending a
/// `remote-subscription-change` message with only an `add` field and no `remove`) doesn't
/// cause the Rust server to drop the connection. Before the fix, the missing `remove` field
/// caused a decode error that terminated the connection.
#[tokio::test]
async fn js_client_with_remote_heads_subscription_can_sync_through_rust_server() {
    init_logging();
    let server = start_rust_server().await;
    let js = JsWrapper::create().await.unwrap();

    // This JS client enables remote heads gossiping and subscribes to a storage ID.
    // When it connects, it sends a `remote-subscription-change` message with only `add`
    // (no `remove` field) to the Rust server.
    let (doc_id, heads, _child1) = js
        .subscribe_and_create_doc(server.port, "1fcd2698-3426-4288-9c47-85364db5073b")
        .await
        .unwrap();

    // If the Rust server choked on the subscription message and dropped the connection,
    // the document won't have been synced and this fetch will fail.
    let fetched_heads = js.fetch_doc(server.port, doc_id).await.unwrap();

    assert_eq!(heads, fetched_heads);
}

/// Test that a JS client sending `remote-heads-changed` messages (which contain timestamps
/// encoded as CBOR float64 by cbor-x) doesn't cause the Rust server to drop the connection.
///
/// The setup: a JS client first syncs a document with a JS server (which has a storage ID),
/// storing remote heads info with a `Date.now()` timestamp. It then connects to the Rust server,
/// which becomes a "generous peer", triggering `addGenerousPeer` to send a `remote-heads-changed`
/// message to the Rust server with the f64 timestamp.
#[tokio::test]
async fn js_client_sending_remote_heads_changed_does_not_break_rust_server() {
    init_logging();
    let js = JsWrapper::create().await.unwrap();
    let js_server = js.start_server().await.unwrap();
    let rust_server = start_rust_server().await;

    // This JS client first syncs with the JS server (building up remote heads info),
    // then connects to the Rust server, which triggers a `remote-heads-changed` message
    // with a float64-encoded timestamp being sent to the Rust server.
    let (doc_id, heads, _child1) = js
        .create_and_relay_heads(js_server.port, rust_server.port)
        .await
        .unwrap();

    // If the Rust server choked on the remote-heads-changed message (f64 timestamp),
    // the document won't have been synced and this fetch will fail.
    let fetched_heads = js.fetch_doc(rust_server.port, doc_id).await.unwrap();

    assert_eq!(heads, fetched_heads);
}

/// Test that the JS server saves sync state for a non-ephemeral samod peer.
///
/// When samod connects with `isEphemeral: false` and a `storageId`, the JS
/// automerge-repo should persist sync state keyed by that storage ID. If this
/// doesn't happen, reconnecting peers will have to re-sync from scratch,
/// resulting in unnecessarily large initial sync messages.
#[tokio::test]
async fn js_server_saves_sync_state_for_non_ephemeral_samod_peer() {
    init_logging();
    let js = JsWrapper::create().await.unwrap();
    let js_server = js.start_server().await.unwrap();
    let port = js_server.port;

    let repo = samod_connected_to_js_server(port, Some("repo1".to_string())).await;

    let doc_handle = repo.create(Automerge::new()).await.unwrap();
    doc_handle
        .with_document(|doc| {
            doc.transact(|tx| {
                tx.put(automerge::ROOT, "key", "value")?;
                Ok::<_, automerge::AutomergeError>(())
            })
        })
        .unwrap();

    // Wait for sync to complete and sync state to be persisted
    tokio::time::sleep(Duration::from_millis(2000)).await;

    let keys = js_server.storage_keys().await.unwrap();
    println!("JS server storage keys: {:?}", keys);

    // The JS server should have saved sync state for the samod peer.
    // Sync state keys have the form [documentId, "sync-state", storageId].
    let has_sync_state = keys
        .iter()
        .any(|key| key.len() >= 2 && key[1] == "sync-state");
    assert!(
        has_sync_state,
        "JS server should have saved sync state for the non-ephemeral samod peer, but storage keys were: {:?}",
        keys
    );
}

async fn samod_connected_to_js_server(port: u16, peer_id: Option<String>) -> Repo {
    let mut builder = Repo::build_tokio();
    if let Some(peer_id) = peer_id {
        builder = builder.with_peer_id(PeerId::from(peer_id.as_str()));
    }
    let repo = builder.load().await;
    let url = Url::parse(&format!("ws://localhost:{}", port)).unwrap();

    let dialer_handle = repo.dial_websocket(url, BackoffConfig::default()).unwrap();

    tokio::time::timeout(Duration::from_secs(5), dialer_handle.established())
        .await
        .expect("dial_websocket timed out")
        .expect("dial_websocket failed");

    repo
}

struct RunningRustServer {
    port: u16,
    #[allow(dead_code)]
    handle: Repo,
    #[allow(dead_code)]
    acceptor: AcceptorHandle,
}

async fn start_rust_server() -> RunningRustServer {
    let handle = Repo::build_tokio().load().await;
    let listener = TcpListener::bind("0.0.0.0:0")
        .await
        .expect("unable to bind socket");
    let port = listener.local_addr().unwrap().port();
    let url = Url::parse(&format!("ws://0.0.0.0:{}", port)).unwrap();
    let acceptor = handle.make_acceptor(url).unwrap();
    let app = axum::Router::new()
        .route("/", axum::routing::get(websocket_handler))
        .with_state(acceptor.clone());
    let server = axum::serve(listener, app).into_future();
    tokio::spawn(server);
    RunningRustServer {
        port,
        handle,
        acceptor,
    }
}

async fn websocket_handler(
    ws: axum::extract::ws::WebSocketUpgrade,
    axum::extract::State(acceptor): axum::extract::State<AcceptorHandle>,
) -> axum::response::Response {
    ws.on_upgrade(|socket| async move {
        if let Err(e) = acceptor.accept_axum(socket) {
            tracing::error!(?e, "failed to accept axum websocket");
        }
    })
}
