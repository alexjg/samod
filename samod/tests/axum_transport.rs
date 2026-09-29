#![cfg(all(feature = "axum", feature = "tokio", feature = "tungstenite"))]

use std::time::Duration;

use axum::{Router, extract::ws::WebSocketUpgrade, routing::get};
use futures::StreamExt;
use samod::{AcceptorEvent, BackoffConfig, DialerFailed, PeerId, Repo, Transport};
use tokio::time::timeout;

const TIMEOUT: Duration = Duration::from_secs(5);

async fn check_websocket_connection(use_constructor: bool, expected: Option<&str>) {
    let server = Repo::build_tokio()
        .with_peer_id("server".into())
        .load()
        .await;
    let client = Repo::build_tokio()
        .with_peer_id("client".into())
        .load()
        .await;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url: url::Url = format!("ws://{}/", listener.local_addr().unwrap())
        .parse()
        .unwrap();
    let acceptor = server.make_acceptor(url.clone()).unwrap();
    let mut events = acceptor.events();
    let server_acceptor = acceptor.clone();
    let expected_peer_id = expected.map(PeerId::from);
    let app = Router::new().route(
        "/",
        get(move |upgrade: WebSocketUpgrade| {
            let acceptor = server_acceptor.clone();
            let expected_peer_id = expected_peer_id.clone();
            async move {
                upgrade.on_upgrade(move |socket| async move {
                    if use_constructor {
                        let mut transport = Transport::from_axum(socket);
                        if let Some(peer_id) = expected_peer_id {
                            transport = transport.with_expected_peer_id(peer_id);
                        }
                        acceptor.accept(transport).unwrap();
                    } else {
                        acceptor.accept_axum(socket, expected_peer_id).unwrap();
                    }
                })
            }
        }),
    );
    let http_server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let dialer = client
        .dial_websocket(
            url,
            BackoffConfig {
                max_retries: Some(0),
                ..BackoffConfig::default()
            },
        )
        .unwrap();

    let result = timeout(TIMEOUT, dialer.established()).await.unwrap();
    let event = timeout(TIMEOUT, events.next()).await.unwrap().unwrap();
    if expected.is_none() || expected == Some("client") {
        assert_eq!(result.unwrap().peer_id, PeerId::from("server"));
        assert!(
            matches!(event, AcceptorEvent::ClientConnected { peer_info, .. } if peer_info.peer_id == PeerId::from("client"))
        );
        assert_eq!(acceptor.connection_count(), 1);
        // Exercise both directions of the WebSocket byte adapter, not just the handshake.
        let doc = server.create(automerge::Automerge::new()).await.unwrap();
        let found = timeout(TIMEOUT, client.find(doc.document_id().clone()))
            .await
            .unwrap()
            .unwrap();
        assert!(found.is_some());
    } else {
        assert!(matches!(result, Err(DialerFailed::MaxRetriesReached)));
        assert!(!dialer.is_connected());
        assert_eq!(acceptor.connection_count(), 0);
        match event {
            AcceptorEvent::ClientDisconnected { reason, .. } => {
                assert!(
                    reason
                        .to_string()
                        .contains("peer ID mismatch: expected someone-else, received client")
                );
            }
            other => panic!("unexpected event: {other:?}"),
        }
    }

    client.stop().await;
    server.stop().await;
    http_server.abort();
    let _ = http_server.await;
}

#[tokio::test]
async fn from_axum_supports_pinned_and_unpinned_transports() {
    for expected in [None, Some("client"), Some("someone-else")] {
        check_websocket_connection(true, expected).await;
    }
}

#[tokio::test]
async fn accept_axum_supports_optional_expected_peer_id() {
    for expected in [None, Some("client"), Some("someone-else")] {
        check_websocket_connection(false, expected).await;
    }
}
