#![cfg(feature = "tokio")]

//! Regression test for https://github.com/paulsonnentag/automerge-rust-sync-server/issues/8
//!
//! When a connection's underlying transport errors on `send` and then also
//! errors on `close`, the `SinkMapErr` wrapper from `futures-util` panics
//! with "polled MapErr after completion" because its error-mapping closure
//! is `FnOnce` and was already consumed by the first error.
//!
//! The panic occurs inside `drive_connection` in `io_loop.rs`:
//!
//! 1. `sink.send(msg).await` fails → `SinkMapErr::take_f()` consumes `f`
//! 2. `sink.close().await` fails → `SinkMapErr::take_f()` panics (f is None)

use std::{
    pin::Pin,
    sync::Arc,
    task::{Context, Poll},
    time::Duration,
};

use futures::{Sink, SinkExt, StreamExt};
use samod::{AcceptorHandle, BackoffConfig, Dialer, PeerId, Repo, Transport};
use tokio_stream::wrappers::ReceiverStream;
use url::Url;

fn init_logging() {
    let _ = tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .try_init();
}

// =============================================================================
// A sink that errors on both send and close
// =============================================================================

#[derive(Debug)]
struct FaultyError(String);

impl std::fmt::Display for FaultyError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl std::error::Error for FaultyError {}

/// A sink that accepts one `start_send` but errors on `poll_flush` (simulating
/// a connection timeout during send), and then also errors on `poll_close`
/// (simulating the underlying connection being truly broken).
///
/// When this sink is wrapped by `Transport::new` (which applies `sink_map_err`),
/// the first error consumes the `FnOnce` error mapper. The second error
/// (on close) then triggers the "polled MapErr after completion" panic.
struct FailOnSendAndCloseSink {
    send_errored: bool,
}

impl FailOnSendAndCloseSink {
    fn new() -> Self {
        Self {
            send_errored: false,
        }
    }
}

impl Unpin for FailOnSendAndCloseSink {}

impl Sink<Vec<u8>> for FailOnSendAndCloseSink {
    type Error = FaultyError;

    fn poll_ready(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }

    fn start_send(self: Pin<&mut Self>, _item: Vec<u8>) -> Result<(), Self::Error> {
        Ok(())
    }

    fn poll_flush(
        mut self: Pin<&mut Self>,
        _cx: &mut Context<'_>,
    ) -> Poll<Result<(), Self::Error>> {
        if !self.send_errored {
            self.send_errored = true;
            Poll::Ready(Err(FaultyError("Connection timed out".into())))
        } else {
            Poll::Ready(Ok(()))
        }
    }

    fn poll_close(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Err(FaultyError("broken pipe on close".into())))
    }
}

// =============================================================================
// A dialer that injects the faulty sink on the dialer side
// =============================================================================

struct FaultySinkDialer {
    url: Url,
    acceptor: AcceptorHandle,
}

impl Dialer for FaultySinkDialer {
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
        let acceptor = self.acceptor.clone();

        Box::pin(async move {
            // Create a normal channel pair. The acceptor side gets a fully
            // working transport so the hub can run the handshake protocol
            // on that side without problems.
            let (acc_tx, dialer_rx) = tokio::sync::mpsc::channel::<Vec<u8>>(16);
            let (dialer_tx, acc_rx) = tokio::sync::mpsc::channel::<Vec<u8>>(16);

            let acc_stream = ReceiverStream::new(acc_rx).map(Ok::<_, FaultyError>);
            let acc_sink = tokio_util::sync::PollSender::new(acc_tx)
                .sink_map_err(|e| FaultyError(format!("send error: {e:?}")));
            acceptor
                .accept(Transport::new(acc_stream, acc_sink))
                .map_err(|e| Box::new(e) as Box<dyn std::error::Error + Send + Sync + 'static>)?;

            // The dialer side gets a normal stream (so inbound messages from
            // the acceptor arrive fine) but a FAULTY sink.
            //
            // We drop dialer_tx since the faulty sink won't actually deliver
            // anything to the acceptor anyway.
            drop(dialer_tx);

            let dialer_stream = ReceiverStream::new(dialer_rx).map(Ok::<_, FaultyError>);
            let faulty_sink = FailOnSendAndCloseSink::new();

            // Transport::new wraps the sink in `sink_map_err` — that's the
            // wrapper whose `FnOnce` error-mapper panics on a second error.
            Ok(Transport::new(dialer_stream, faulty_sink))
        })
    }
}

// =============================================================================
// The test
// =============================================================================

/// Regression test for
/// https://github.com/paulsonnentag/automerge-rust-sync-server/issues/8
///
/// When the underlying transport errors on send and then also errors on
/// close, `drive_connection` must not panic. Before the fix the
/// `SinkMapErr` wrapper would panic with "polled MapErr after completion"
/// because its `FnOnce` error-mapper was already consumed by the send error.
///
/// NOTE: The panic occurs inside a spawned tokio task (the io_loop). Tokio
/// catches panics in spawned tasks so they don't automatically propagate
/// to the test thread. We therefore install a custom panic hook that
/// records the panic message so we can assert on it after the fact. This
/// is unfortunate — ideally `Repo` would expose an API for observing
/// background task health (e.g. via a channel or health-check method) so
/// that consumers and tests could detect io_loop failures without
/// resorting to panic hooks. That is left as future work.
#[tokio::test]
async fn sink_send_error_then_close_error_does_not_panic() {
    init_logging();

    // Track whether a panic occurred in a spawned task.  We need this
    // because tokio silently catches panics in spawned tasks — without the
    // hook the test would pass even though the io_loop panicked.
    let panicked: Arc<std::sync::Mutex<Option<String>>> = Arc::new(std::sync::Mutex::new(None));

    let panicked_hook = panicked.clone();
    let prev_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let msg = if let Some(s) = info.payload().downcast_ref::<&str>() {
            s.to_string()
        } else if let Some(s) = info.payload().downcast_ref::<String>() {
            s.clone()
        } else {
            info.to_string()
        };
        *panicked_hook.lock().unwrap() = Some(msg);
    }));

    let alice = Repo::build_tokio()
        .with_peer_id(PeerId::from("alice"))
        .load()
        .await;

    let bob = Repo::build_tokio()
        .with_peer_id(PeerId::from("bob"))
        .load()
        .await;

    let url = Url::parse("ws://test-faulty-sink:0").unwrap();
    let acceptor = bob.make_acceptor(url.clone()).unwrap();

    let dialer = FaultySinkDialer { url, acceptor };

    // Start dialing — the io_loop will call drive_connection with our faulty
    // transport. The hub will try to send a handshake (Join) message, which
    // will trigger the send error, then drive_connection calls sink.close()
    // which also errors. Before the fix this would panic.
    let _handle = alice
        .dial(BackoffConfig::default(), Arc::new(dialer))
        .unwrap();

    // Give the io_loop time to process the connection and hit the error path.
    tokio::time::sleep(Duration::from_millis(500)).await;

    // Restore the previous panic hook before asserting.
    std::panic::set_hook(prev_hook);

    if let Some(msg) = panicked.lock().unwrap().take() {
        panic!("spawned task panicked: {msg}");
    }
}
