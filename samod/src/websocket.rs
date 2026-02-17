#[cfg(any(feature = "tungstenite", feature = "axum"))]
use futures::TryStreamExt;
use futures::{Sink, SinkExt, Stream, StreamExt};

use crate::Repo;

#[cfg(feature = "tungstenite")]
use std::pin::Pin;
#[cfg(feature = "tungstenite")]
use std::sync::Arc;
#[cfg(feature = "tungstenite")]
use url::Url;

/// A copy of tungstenite::Message
///
/// This is necessary because axum uses tungstenite::Message internally but exposes it's own
/// version so in order to have the logic which handles tungstenite clients and axum servers
/// written in the same function we have to map both the tungstenite `Message` and the axum
/// `Message` to our own type.
pub enum WsMessage {
    Binary(Vec<u8>),
    Text(String),
    Close,
    Ping(Vec<u8>),
    Pong(Vec<u8>),
}

#[cfg(feature = "tungstenite")]
impl From<WsMessage> for tungstenite::Message {
    fn from(msg: WsMessage) -> Self {
        match msg {
            WsMessage::Binary(data) => tungstenite::Message::Binary(data.into()),
            WsMessage::Text(data) => tungstenite::Message::Text(data.into()),
            WsMessage::Close => tungstenite::Message::Close(None),
            WsMessage::Ping(data) => tungstenite::Message::Ping(data.into()),
            WsMessage::Pong(data) => tungstenite::Message::Pong(data.into()),
        }
    }
}

#[cfg(feature = "tungstenite")]
impl From<tungstenite::Message> for WsMessage {
    fn from(msg: tungstenite::Message) -> Self {
        match msg {
            tungstenite::Message::Binary(data) => WsMessage::Binary(data.into()),
            tungstenite::Message::Text(data) => WsMessage::Text(data.as_str().to_string()),
            tungstenite::Message::Close(_) => WsMessage::Close,
            tungstenite::Message::Ping(data) => WsMessage::Ping(data.into()),
            tungstenite::Message::Pong(data) => WsMessage::Pong(data.into()),
            tungstenite::Message::Frame(_) => unreachable!("unexpected frame message"),
        }
    }
}

#[cfg(feature = "axum")]
impl From<WsMessage> for axum::extract::ws::Message {
    fn from(msg: WsMessage) -> Self {
        match msg {
            WsMessage::Binary(data) => axum::extract::ws::Message::Binary(data.into()),
            WsMessage::Text(data) => axum::extract::ws::Message::Text(data.into()),
            WsMessage::Close => axum::extract::ws::Message::Close(None),
            WsMessage::Ping(data) => axum::extract::ws::Message::Ping(data.into()),
            WsMessage::Pong(data) => axum::extract::ws::Message::Pong(data.into()),
        }
    }
}

#[cfg(feature = "axum")]
impl From<axum::extract::ws::Message> for WsMessage {
    fn from(msg: axum::extract::ws::Message) -> Self {
        match msg {
            axum::extract::ws::Message::Binary(data) => WsMessage::Binary(data.into()),
            axum::extract::ws::Message::Text(data) => WsMessage::Text(data.as_str().to_string()),
            axum::extract::ws::Message::Close(_) => WsMessage::Close,
            axum::extract::ws::Message::Ping(data) => WsMessage::Ping(data.into()),
            axum::extract::ws::Message::Pong(data) => WsMessage::Pong(data.into()),
        }
    }
}

type BoxedBytesStream = futures::stream::BoxStream<'static, Result<Vec<u8>, NetworkError>>;

/// Convert a WebSocket stream/sink into a raw bytes stream/sink.
///
/// This is the shared transformation used by both `connect_websocket` and
/// the accept convenience methods. It filters out non-binary messages,
/// wraps outbound bytes in `WsMessage::Binary`, and maps errors to
/// `NetworkError`.
fn ws_to_bytes<S, M>(
    stream: S,
) -> (
    BoxedBytesStream,
    impl Sink<Vec<u8>, Error = NetworkError> + Send + Unpin,
)
where
    M: Into<WsMessage> + From<WsMessage> + Send + 'static,
    S: Sink<M, Error = NetworkError> + Stream<Item = Result<M, NetworkError>> + Send + 'static,
{
    let (sink, stream) = stream.split();

    let msg_stream = stream
        .filter_map::<_, Result<Vec<u8>, NetworkError>, _>({
            move |msg| async move {
                let msg = match msg {
                    Ok(m) => m,
                    Err(e) => {
                        return Some(Err(NetworkError(format!("websocket receive error: {e}"))));
                    }
                };
                match msg.into() {
                    WsMessage::Binary(data) => Some(Ok(data)),
                    WsMessage::Close => {
                        tracing::debug!("websocket closing");
                        None
                    }
                    WsMessage::Ping(_) | WsMessage::Pong(_) => None,
                    WsMessage::Text(_) => Some(Err(NetworkError(
                        "unexpected string message on websocket".to_string(),
                    ))),
                }
            }
        })
        .boxed();

    let msg_sink = sink
        .sink_map_err(|e| NetworkError(format!("websocket send error: {e}")))
        .with(|msg| futures::future::ready(Ok::<_, NetworkError>(WsMessage::Binary(msg).into())));

    (msg_stream, msg_sink)
}

pub struct NetworkError(String);
impl std::fmt::Debug for NetworkError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}
impl std::fmt::Display for NetworkError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}
impl std::error::Error for NetworkError {}

// --- TungsteniteDialer ---

/// A [`Dialer`](crate::Dialer) that connects to a WebSocket endpoint via
/// `tokio-tungstenite`.
///
/// Used internally by [`Repo::dial_websocket`] to provide a built-in
/// tungstenite-based dialer with automatic reconnection.
#[cfg(feature = "tungstenite")]
pub struct TungsteniteDialer {
    url: Url,
}

#[cfg(feature = "tungstenite")]
impl TungsteniteDialer {
    /// Create a new `TungsteniteDialer` for the given URL.
    pub fn new(url: Url) -> Self {
        Self { url }
    }
}

#[cfg(feature = "tungstenite")]
impl crate::Dialer for TungsteniteDialer {
    fn url(&self) -> Url {
        self.url.clone()
    }

    fn connect(
        &self,
    ) -> Pin<
        Box<
            dyn std::future::Future<
                    Output = Result<
                        crate::Transport,
                        Box<dyn std::error::Error + Send + Sync + 'static>,
                    >,
                > + Send,
        >,
    > {
        let url = self.url.clone();
        Box::pin(async move {
            let (ws, _response) = tokio_tungstenite::connect_async(url.as_str()).await?;

            // Wrap tungstenite errors into NetworkError
            let ws = ws
                .map_err(|e| NetworkError(format!("error receiving websocket message: {}", e)))
                .sink_map_err(|e| NetworkError(format!("error sending websocket message: {}", e)));

            // Convert WebSocket messages to raw bytes
            let (msg_stream, msg_sink) = ws_to_bytes::<_, tungstenite::Message>(ws);

            Ok(crate::Transport::new(msg_stream, msg_sink))
        })
    }
}

// --- Repo convenience methods ---

impl Repo {
    /// Dial a WebSocket endpoint with automatic reconnection.
    ///
    /// Uses a built-in tungstenite [`Dialer`](crate::Dialer) that connects
    /// via `tokio-tungstenite`. The connection will automatically reconnect
    /// with the provided backoff configuration when the connection is lost.
    ///
    /// This is a convenience wrapper around [`Repo::dial`] with a
    /// [`TungsteniteDialer`].
    ///
    /// # Arguments
    ///
    /// * `url` - The WebSocket URL to connect to (e.g. `"wss://sync.example.com"`).
    /// * `backoff` - Backoff configuration for reconnection attempts.
    ///
    /// # Returns
    ///
    /// A [`DialerHandle`](crate::DialerHandle) for observing and controlling the dialer.
    #[cfg(feature = "tungstenite")]
    pub fn dial_websocket(
        &self,
        url: Url,
        backoff: crate::BackoffConfig,
    ) -> Result<crate::DialerHandle, crate::Stopped> {
        let dialer = Arc::new(TungsteniteDialer::new(url));
        self.dial(backoff, dialer)
    }
}

// --- AcceptorHandle convenience methods ---

impl crate::AcceptorHandle {
    /// Accept a tungstenite WebSocket connection.
    ///
    /// This is a convenience wrapper around [`AcceptorHandle::accept`] that
    /// handles the conversion between tungstenite's `Message` type and raw
    /// bytes.
    ///
    /// # Arguments
    ///
    /// * `socket` - A tungstenite WebSocket (both `Sink` and `Stream`).
    #[cfg(feature = "tungstenite")]
    pub fn accept_tungstenite<S>(&self, socket: S) -> Result<(), crate::Stopped>
    where
        S: Sink<tungstenite::Message, Error = tungstenite::Error>
            + Stream<Item = Result<tungstenite::Message, tungstenite::Error>>
            + Send
            + 'static,
    {
        let ws = socket
            .map_err(|e| NetworkError(format!("error receiving websocket message: {}", e)))
            .sink_map_err(|e| NetworkError(format!("error sending websocket message: {}", e)));
        let (msg_stream, msg_sink) = ws_to_bytes::<_, tungstenite::Message>(ws);
        self.accept(crate::Transport::new(msg_stream, msg_sink))
    }

    /// Accept an axum WebSocket connection.
    ///
    /// This is a convenience wrapper around [`AcceptorHandle::accept`] that
    /// handles the conversion between axum's `Message` type and raw bytes.
    ///
    /// # Arguments
    ///
    /// * `socket` - An axum WebSocket (both `Sink` and `Stream`).
    #[cfg(feature = "axum")]
    pub fn accept_axum(&self, socket: axum::extract::ws::WebSocket) -> Result<(), crate::Stopped> {
        let ws = socket
            .map_err(|e| NetworkError(format!("error receiving websocket message: {}", e)))
            .sink_map_err(|e| NetworkError(format!("error sending websocket message: {}", e)));
        let (msg_stream, msg_sink) = ws_to_bytes::<_, axum::extract::ws::Message>(ws);
        self.accept(crate::Transport::new(msg_stream, msg_sink))
    }
}
