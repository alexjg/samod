use std::{sync::Arc, time::Duration};

use automerge::{Automerge, ReadDoc, transaction::Transactable};
use futures::{StreamExt, lock::Mutex};
use samod::{ConnDirection, PeerId, Repo};

mod js_wrapper;
use js_wrapper::JsWrapper;
use tokio::net::TcpListener;

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

async fn samod_connected_to_js_server(port: u16, peer_id: Option<String>) -> Repo {
    let mut builder = Repo::build_tokio();
    if let Some(peer_id) = peer_id {
        builder = builder.with_peer_id(PeerId::from(peer_id.as_str()));
    }
    let handle = builder.load().await;
    let (conn, _) = tokio_tungstenite::connect_async(format!("ws://localhost:{}", port))
        .await
        .unwrap();

    handle
        .connect_tungstenite(conn, ConnDirection::Outgoing)
        .unwrap();

    handle
}

struct RunningRustServer {
    port: u16,
    #[allow(dead_code)]
    handle: Repo,
    #[allow(dead_code)]
    running_connections: Arc<Mutex<Vec<tokio::task::JoinHandle<()>>>>,
}

async fn start_rust_server() -> RunningRustServer {
    let handle = Repo::build_tokio().load().await;
    let running_connections = Arc::new(Mutex::new(Vec::new()));
    let app = axum::Router::new()
        .route("/", axum::routing::get(websocket_handler))
        .with_state((handle.clone(), running_connections.clone()));
    let listener = TcpListener::bind("0.0.0.0:0")
        .await
        .expect("unable to bind socket");
    let port = listener.local_addr().unwrap().port();
    let server = axum::serve(listener, app).into_future();
    tokio::spawn(server);
    RunningRustServer {
        port,
        handle,
        running_connections,
    }
}

#[allow(clippy::type_complexity)]
async fn websocket_handler(
    ws: axum::extract::ws::WebSocketUpgrade,
    axum::extract::State((handle, running_connections)): axum::extract::State<(
        Repo,
        Arc<Mutex<Vec<tokio::task::JoinHandle<()>>>>,
    )>,
) -> axum::response::Response {
    ws.on_upgrade(|socket| handle_socket(socket, handle, running_connections))
}

async fn handle_socket(
    socket: axum::extract::ws::WebSocket,
    repo: Repo,
    running_connections: Arc<Mutex<Vec<tokio::task::JoinHandle<()>>>>,
) {
    let conn = repo.accept_axum(socket).unwrap();
    let handle = tokio::spawn(async move {
        let finished = conn.finished().await;
        tracing::error!(?finished, "connection finished");
    });
    running_connections.lock().await.push(handle);
}
