#![cfg(feature = "tokio")]

use std::{convert::Infallible, sync::Arc, time::Duration};

use futures::{FutureExt, SinkExt, StreamExt, future::BoxFuture, poll};
use samod::{
    AcceptorEvent, AcceptorHandle, BackoffConfig, DialError, Dialer, DialerEvent, DialerFailed,
    PeerId, Repo, Transport,
};
use tokio::time::timeout;

const TIMEOUT: Duration = Duration::from_secs(5);

struct PinnedDialer {
    acceptor: AcceptorHandle,
    expected_server: PeerId,
    expected_client: PeerId,
}

impl PinnedDialer {
    fn new(acceptor: AcceptorHandle, expected_server: &str, expected_client: &str) -> Arc<Self> {
        Arc::new(Self {
            acceptor,
            expected_server: expected_server.into(),
            expected_client: expected_client.into(),
        })
    }
}

impl Dialer for PinnedDialer {
    type Error = Infallible;

    fn url(&self) -> url::Url {
        "memory://authenticated".parse().unwrap()
    }

    fn connect(&self) -> BoxFuture<'static, Result<Transport, DialError<Self::Error>>> {
        let acceptor = self.acceptor.clone();
        let expected_client = self.expected_client.clone();
        let expected_server = self.expected_server.clone();
        Box::pin(async move {
            let (client, server) = tokio::io::duplex(65536);
            acceptor
                .accept(Transport::from_tokio_io(server).with_expected_peer_id(expected_client))
                .map_err(DialError::transient)?;
            Ok(Transport::from_tokio_io(client).with_expected_peer_id(expected_server))
        })
    }
}

async fn repo(peer_id: &str) -> Repo {
    Repo::build_tokio()
        .with_peer_id(peer_id.into())
        .load()
        .await
}

fn no_retries() -> BackoffConfig {
    BackoffConfig {
        max_retries: Some(0),
        ..BackoffConfig::default()
    }
}

#[tokio::test]
async fn matching_pins_allow_handshake_and_sync() {
    let server = repo("server").await;
    let client = repo("client").await;
    let acceptor = server
        .make_acceptor("memory://server".parse().unwrap())
        .unwrap();
    let handle = client
        .dial(
            no_retries(),
            PinnedDialer::new(acceptor.clone(), "server", "client"),
        )
        .unwrap();

    let peer = timeout(TIMEOUT, handle.established())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(peer.peer_id, PeerId::from("server"));
    assert_eq!(acceptor.connection_count(), 1);

    let doc = server.create(automerge::Automerge::new()).await.unwrap();
    let found = timeout(TIMEOUT, client.find(doc.document_id().clone()))
        .await
        .unwrap()
        .unwrap();
    assert!(found.is_some());

    client.stop().await;
    server.stop().await;
}

#[tokio::test]
async fn dialer_pin_rejects_the_wrong_server() {
    let server = repo("server").await;
    let client = repo("client").await;
    let acceptor = server
        .make_acceptor("memory://server".parse().unwrap())
        .unwrap();
    let handle = client
        .dial(
            no_retries(),
            PinnedDialer::new(acceptor, "trusted-server", "client"),
        )
        .unwrap();
    let mut events = handle.events();

    let result = timeout(TIMEOUT, handle.established()).await.unwrap();
    assert!(matches!(result, Err(DialerFailed::MaxRetriesReached)));
    assert!(!handle.is_connected());
    match timeout(TIMEOUT, events.next()).await.unwrap().unwrap() {
        DialerEvent::Disconnected { reason } => {
            assert!(reason.contains("peer ID mismatch: expected trusted-server, received server"));
        }
        other => panic!("unexpected event: {other:?}"),
    }

    client.stop().await;
    server.stop().await;
}

#[tokio::test]
async fn rejected_handshake_closes_transport_and_resolves_pending_waiters() {
    let server = repo("server").await;
    let acceptor = server
        .make_acceptor("memory://server".parse().unwrap())
        .unwrap();
    let (remote, local) = tokio::io::duplex(65536);
    let connection = acceptor
        .accept(Transport::from_tokio_io(local).with_expected_peer_id("trusted-client".into()))
        .unwrap();
    let mut handshake = connection.handshake_complete().boxed();
    assert!(poll!(&mut handshake).is_pending());
    let mut events = connection.events();
    let mut remote =
        tokio_util::codec::Framed::new(remote, tokio_util::codec::LengthDelimitedCodec::new());
    // CBOR: {"type":"join", "senderId":"impostor", "supportedProtocolVersions":["1"]}
    let join =
        b"\xa3\x64type\x64join\x68senderId\x68impostor\x78\x19supportedProtocolVersions\x81\x611";
    remote.send(join.to_vec().into()).await.unwrap();
    // The protocol error is followed by actual transport EOF, not just removal
    // of the connection from the hub's bookkeeping.
    assert!(
        timeout(TIMEOUT, remote.next())
            .await
            .unwrap()
            .unwrap()
            .is_ok()
    );
    assert!(timeout(TIMEOUT, remote.next()).await.unwrap().is_none());
    assert!(timeout(TIMEOUT, handshake).await.unwrap().is_err());
    for _ in 0..2 {
        assert!(
            timeout(TIMEOUT, connection.handshake_complete())
                .await
                .unwrap()
                .is_err()
        );
    }
    match timeout(TIMEOUT, events.next()).await.unwrap().unwrap() {
        AcceptorEvent::ClientDisconnected { reason, .. } => {
            assert!(reason.to_string().contains("peer ID mismatch"));
        }
        other => panic!("unexpected event: {other:?}"),
    }
    server.stop().await;
}

#[tokio::test]
async fn acceptor_pin_rejects_impostors_without_affecting_healthy_connections() {
    let server = repo("server").await;
    let client = repo("client").await;
    let impostor = repo("impostor").await;
    let acceptor = server
        .make_acceptor("memory://server".parse().unwrap())
        .unwrap();
    let good = client
        .dial(
            no_retries(),
            PinnedDialer::new(acceptor.clone(), "server", "client"),
        )
        .unwrap();
    timeout(TIMEOUT, good.established()).await.unwrap().unwrap();
    assert_eq!(acceptor.connection_count(), 1);

    let mut events = acceptor.events();
    let bad = impostor
        .dial(
            no_retries(),
            PinnedDialer::new(acceptor.clone(), "server", "client"),
        )
        .unwrap();
    let result = timeout(TIMEOUT, bad.established()).await.unwrap();
    assert!(matches!(result, Err(DialerFailed::MaxRetriesReached)));
    assert!(!bad.is_connected());
    match timeout(TIMEOUT, events.next()).await.unwrap().unwrap() {
        AcceptorEvent::ClientDisconnected { reason, .. } => {
            assert!(
                reason
                    .to_string()
                    .contains("peer ID mismatch: expected client, received impostor")
            );
        }
        other => panic!("unexpected event: {other:?}"),
    }
    assert_eq!(acceptor.connection_count(), 1);
    assert!(good.is_connected());

    impostor.stop().await;
    client.stop().await;
    server.stop().await;
}
