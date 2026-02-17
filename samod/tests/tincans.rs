#![allow(dead_code)]
use std::{pin::Pin, sync::Arc};

use futures::{FutureExt, Sink, SinkExt, Stream, StreamExt, select};
use rand::Rng;
use samod::{
    AcceptorEvent, AcceptorHandle, BackoffConfig, ConnectionId, Dialer, DialerHandle, PeerInfo,
    Repo, Transport,
};
use tokio_stream::wrappers::ReceiverStream;
use tokio_util::sync::{CancellationToken, PollSender};
use url::Url;

struct TinCanError(String);
impl std::fmt::Display for TinCanError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}
impl std::fmt::Debug for TinCanError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}
impl std::error::Error for TinCanError {}

/// One side of an in-memory transport.
struct MemTransportSide {
    send: Box<dyn Send + Unpin + Sink<Vec<u8>, Error = TinCanError>>,
    recv: Box<dyn Send + Unpin + Stream<Item = Result<Vec<u8>, TinCanError>>>,
}

/// Create a pair of in-memory transport sides connected through a cancellable middle relay.
///
/// Returns `(left_side, right_side)`. When the cancel token is triggered, the
/// middle relay drops its ends, causing both sides to observe stream closure.
fn mem_transport_pair_with_cancel(
    cancel: CancellationToken,
) -> (MemTransportSide, MemTransportSide) {
    // left <-> middle_left <-> middle_right <-> right
    let (left_tx, middle_left_rx) = tokio::sync::mpsc::channel::<Vec<u8>>(16);
    let (middle_left_tx, left_rx) = tokio::sync::mpsc::channel::<Vec<u8>>(16);
    let (right_tx, middle_right_rx) = tokio::sync::mpsc::channel::<Vec<u8>>(16);
    let (middle_right_tx, right_rx) = tokio::sync::mpsc::channel::<Vec<u8>>(16);

    // Spawn the middle relay task
    let mut middle_recv_left = ReceiverStream::new(middle_left_rx);
    let mut middle_recv_right = ReceiverStream::new(middle_right_rx);
    let mut middle_send_left =
        PollSender::new(middle_left_tx).sink_map_err(|e| TinCanError(format!("send error: {e:?}")));
    let mut middle_send_right = PollSender::new(middle_right_tx)
        .sink_map_err(|e| TinCanError(format!("send error: {e:?}")));

    tokio::spawn(async move {
        loop {
            select! {
                next = middle_recv_left.next().fuse() => {
                    let Some(msg) = next else { break };
                    if middle_send_right.send(msg).await.is_err() { break }
                }
                next = middle_recv_right.next().fuse() => {
                    let Some(msg) = next else { break };
                    if middle_send_left.send(msg).await.is_err() { break }
                }
                _ = cancel.cancelled().fuse() => {
                    break;
                }
            }
        }
        tracing::info!("middle relay task finished");
    });

    let left = MemTransportSide {
        send: Box::new(
            PollSender::new(left_tx).sink_map_err(|e| TinCanError(format!("send error: {e:?}"))),
        ),
        recv: Box::new(ReceiverStream::new(left_rx).map(Ok)),
    };

    let right = MemTransportSide {
        send: Box::new(
            PollSender::new(right_tx).sink_map_err(|e| TinCanError(format!("send error: {e:?}"))),
        ),
        recv: Box::new(ReceiverStream::new(right_rx).map(Ok)),
    };

    (left, right)
}

/// A mock dialer that connects to a target `AcceptorHandle` via in-memory
/// channels, routed through a cancellable middle relay.
struct CancellableDialer {
    url: Url,
    acceptor: AcceptorHandle,
    cancel: CancellationToken,
}

impl Dialer for CancellableDialer {
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
        let (dialer_side, acceptor_side) = mem_transport_pair_with_cancel(self.cancel.clone());
        let acceptor = self.acceptor.clone();

        Box::pin(async move {
            acceptor
                .accept(Transport::new(acceptor_side.recv, acceptor_side.send))
                .map_err(|e| Box::new(e) as Box<dyn std::error::Error + Send + Sync + 'static>)?;

            Ok(Transport::new(dialer_side.recv, dialer_side.send))
        })
    }
}

/// The result of connecting two repos.
pub(crate) struct Connected {
    /// Dialer handle (left side). Can be used to get connection_id, peer_info, etc.
    pub dialer_handle: DialerHandle,
    /// Connection ID on the left (dialer) side.
    pub left_connection_id: ConnectionId,
    /// Connection ID on the right (acceptor) side.
    pub right_connection_id: ConnectionId,
    /// Left side peer info as seen by right.
    pub left_peer_info: PeerInfo,
    /// Right side peer info as seen by left.
    pub right_peer_info: PeerInfo,
    /// Cancellation token to simulate connection loss.
    cancel: CancellationToken,
}

impl Connected {
    pub async fn disconnect(self) {
        self.cancel.cancel();
        // Give the relay a moment to shut down
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
}

/// Connect two repos using the dialer/acceptor API.
///
/// `left` acts as the dialer, `right` acts as the acceptor. The connection
/// goes through a cancellable middle relay so it can be disconnected via
/// `Connected::disconnect()`.
///
/// This function waits for both sides to complete the handshake before
/// returning.
pub(crate) async fn connect_repos(left: &Repo, right: &Repo) -> Connected {
    let cancel = CancellationToken::new();

    let id: u64 = rand::rng().random();
    let url = Url::parse(&format!("ws://test-tincans-{}:0", id)).unwrap();
    let acceptor = right.make_acceptor(url.clone()).unwrap();
    let mut acceptor_events = acceptor.events();

    let dialer = CancellableDialer {
        url,
        acceptor: acceptor.clone(),
        cancel: cancel.clone(),
    };

    let dialer_handle = left
        .dial(BackoffConfig::default(), Arc::new(dialer))
        .unwrap();

    // Wait for the dialer side to establish
    let right_peer_info = tokio::time::timeout(
        std::time::Duration::from_secs(5),
        dialer_handle.established(),
    )
    .await
    .expect("dialer established timed out")
    .expect("dialer established failed");

    let left_connection_id = dialer_handle
        .connection_id()
        .expect("dialer should have a connection_id after established");

    // Wait for the acceptor side event
    let (right_connection_id, left_peer_info) =
        tokio::time::timeout(std::time::Duration::from_secs(5), async {
            while let Some(event) = acceptor_events.next().await {
                match event {
                    AcceptorEvent::ClientConnected {
                        peer_info,
                        connection_id,
                    } => return (connection_id, peer_info),
                    _ => continue,
                }
            }
            panic!("acceptor event stream ended without ClientConnected");
        })
        .await
        .expect("acceptor client connected timed out");

    Connected {
        dialer_handle,
        left_connection_id,
        right_connection_id,
        left_peer_info,
        right_peer_info,
        cancel,
    }
}
