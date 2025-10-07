use futures::{Future, Sink, SinkExt, Stream, StreamExt};
#[cfg(feature = "wasm")]
use futures::{
    FutureExt,
    channel::{mpsc, oneshot},
};
#[cfg(feature = "wasm")]
use js_sys::Uint8Array;
#[cfg(feature = "wasm")]
use std::task::Poll;
#[cfg(feature = "wasm")]
use std::{cell::RefCell, pin::Pin, rc::Rc};
#[cfg(feature = "wasm")]
use wasm_bindgen::{JsCast, prelude::Closure};
#[cfg(feature = "wasm")]
use web_sys::{CloseEvent, ErrorEvent, MessageEvent, WebSocket};

use crate::{ConnDirection, ConnFinishedReason, Repo};

#[cfg(feature = "wasm")]
use futures::future::LocalBoxFuture;

#[cfg(feature = "wasm")]
pub struct WasmConnectionEvents {
    pub on_open: oneshot::Receiver<()>,
    pub on_ready: oneshot::Receiver<()>,
    pub finished: LocalBoxFuture<'static, ConnFinishedReason>,
}

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

impl Repo {
    /// Connect a tungstenite websocket
    #[cfg(feature = "tungstenite")]
    pub fn connect_tungstenite<S>(
        &self,
        socket: S,
        direction: ConnDirection,
    ) -> impl Future<Output = ConnFinishedReason> + 'static
    where
        S: Sink<tungstenite::Message, Error = tungstenite::Error>
            + Stream<Item = Result<tungstenite::Message, tungstenite::Error>>
            + Send
            + 'static,
    {
        use futures::stream::TryStreamExt;
        let stream = socket
            .map_err(|e| NetworkError(format!("error receiving websocket message: {}", e)))
            .sink_map_err(|e| NetworkError(format!("error sending websocket message: {}", e)));
        self.connect_websocket(stream, direction)
    }

    /// Accept a websocket in an axum handler
    #[cfg(feature = "axum")]
    pub fn accept_axum<S>(&self, stream: S) -> impl Future<Output = ConnFinishedReason> + 'static
    where
        S: Sink<axum::extract::ws::Message, Error = axum::Error>
            + Stream<Item = Result<axum::extract::ws::Message, axum::Error>>
            + Send
            + 'static,
    {
        use futures::TryStreamExt;

        let stream = stream
            .map_err(|e| NetworkError(format!("error receiving websocket message: {}", e)))
            .sink_map_err(|e| NetworkError(format!("error sending websocket message: {}", e)));
        self.connect_websocket(stream, ConnDirection::Incoming)
    }

    /// Connect to a WebSocket server from a WASM environment
    ///
    /// This method creates a WebSocket connection using the browser's native WebSocket API
    /// and integrates it with samod's synchronization protocol.
    ///
    /// # Arguments
    ///
    /// * `url` - The WebSocket URL to connect to (e.g., "ws://localhost:8080" or "wss://example.com")
    /// * `direction` - Whether this is an outgoing or incoming connection
    ///
    /// # Returns
    ///
    /// A `ConnFinishedReason` indicating how the connection terminated:
    /// - `ConnFinishedReason::Shutdown` - The repo was shut down
    /// - `ConnFinishedReason::Error(String)` - An error occurred
    /// - Other variants as per normal connection lifecycle
    ///
    /// # Example
    ///
    /// ```no_run
    /// use samod::{Repo, ConnDirection};
    ///
    /// let repo = Repo::build_wasm().load().await;
    /// let result = repo.connect_wasm_websocket(
    ///     "ws://localhost:8080/sync",
    ///     ConnDirection::Outgoing
    /// ).await;
    /// ```
    ///
    /// # Panics
    ///
    /// This method must be called from within a WASM environment with access to
    /// the browser's WebSocket API. It will panic if called outside of a browser context.
    #[cfg(feature = "wasm")]
    pub async fn connect_wasm_websocket(
        &self,
        url: &str,
        direction: ConnDirection,
    ) -> ConnFinishedReason {
        tracing::info!("WASM: Attempting WebSocket connection to {}", url);
        tracing::debug!("WASM: Connection direction: {:?}", direction);

        match WasmWebSocket::connect(url).await {
            Ok(ws) => {
                tracing::info!(
                    "WASM: WebSocket connection established, starting protocol handshake"
                );
                let (sink, stream) = ws.split();
                let result = self.connect(stream, sink, direction).await;
                tracing::info!("WASM: Connection finished with reason: {:?}", result);
                result
            }
            Err(e) => {
                let error_msg = format!("Failed to connect WebSocket: {}", e);
                tracing::error!("WASM: {}", error_msg);
                ConnFinishedReason::Error(error_msg)
            }
        }
    }

    #[cfg(feature = "wasm")]
    pub fn connect_wasm_websocket_observable(
        &self,
        url: &str,
        direction: ConnDirection,
    ) -> WasmConnectionEvents {
        let (open_tx, open_rx) = oneshot::channel();
        let (ready_tx, ready_rx) = oneshot::channel();

        let repo = self.clone();
        let url = url.to_string();

        let finished = async move {
            tracing::info!("WASM: Attempting WebSocket connection to {}", url);

            match WasmWebSocket::connect(&url).await {
                Ok(ws) => {
                    tracing::info!("WASM: WebSocket connection established");
                    let _ = open_tx.send(());

                    let (sink, stream) = ws.split();

                    let result = repo
                        .connect_with_ready_signal(stream, sink, direction, ready_tx)
                        .await;

                    tracing::info!("WASM: Connection finished with reason: {:?}", result);
                    result
                }
                Err(e) => {
                    let error_msg = format!("Failed to connect WebSocket: {}", e);
                    tracing::error!("WASM: {}", error_msg);
                    ConnFinishedReason::Error(error_msg)
                }
            }
        }
        .boxed_local();

        WasmConnectionEvents {
            on_open: open_rx,
            on_ready: ready_rx,
            finished,
        }
    }

    /// Connect any stream of [`WsMessage`]s
    ///
    /// [`WsMessage`] is a copy of `tungstenite::Message` and
    /// `axum::extract::ws::Message` which is reimplemented in this crate
    /// because both `tungstenite` and `axum` use their own message types which
    /// are identical, but not the same type. This function allows us to
    /// implement the connection logic once and use it for both `tungstenite`
    /// and `axum`.
    pub fn connect_websocket<S, M>(
        &self,
        stream: S,
        direction: ConnDirection,
    ) -> impl Future<Output = ConnFinishedReason> + 'static
    where
        M: Into<WsMessage> + From<WsMessage> + 'static,
        S: Sink<M, Error = NetworkError> + Stream<Item = Result<M, NetworkError>> + 'static,
    {
        let (sink, stream) = stream.split();

        let msg_stream = stream
            .filter_map::<_, Result<Vec<u8>, NetworkError>, _>({
                move |msg| async move {
                    let msg = match msg {
                        Ok(m) => m,
                        Err(e) => {
                            return Some(Err(NetworkError(format!(
                                "websocket receive error: {e}"
                            ))));
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
            .boxed_local();

        let msg_sink = sink
            .sink_map_err(|e| NetworkError(format!("websocket send error: {e}")))
            .with(|msg| {
                futures::future::ready(Ok::<_, NetworkError>(WsMessage::Binary(msg).into()))
            });

        self.connect(msg_stream, msg_sink, direction)
    }
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

// Hold closures to prevent them from being dropped
#[cfg(feature = "wasm")]
struct ClosureHandlers {
    _on_message: Closure<dyn FnMut(MessageEvent)>,
    _on_close: Closure<dyn FnMut(CloseEvent)>,
    _on_error: Closure<dyn FnMut(ErrorEvent)>,
    _on_open: Closure<dyn FnMut(web_sys::Event)>,
}

/// A WebSocket implementation for WASM environments
///
/// This struct wraps the browser's native WebSocket API and implements
/// the `Stream` and `Sink` traits to work with samod's connection protocol.
///
/// The WebSocket is configured to:
/// - Use binary messages (ArrayBuffer) for data transfer
/// - Automatically handle connection lifecycle events
/// - Convert browser events into a Stream/Sink interface
///
/// # Safety
///
/// This type implements `Send` even though it contains `Rc` and browser objects
/// because WASM is single-threaded. All operations happen on the same thread.
#[cfg(feature = "wasm")]
pub struct WasmWebSocket {
    ws: WebSocket,
    _closures: ClosureHandlers,
    receiver: mpsc::UnboundedReceiver<WsMessage>,
}

#[cfg(feature = "wasm")]
impl WasmWebSocket {
    /// Create a new WebSocket connection
    ///
    /// This method establishes a WebSocket connection to the specified URL and
    /// waits for the connection to be fully established before returning.
    ///
    /// # Arguments
    ///
    /// * `url` - The WebSocket URL to connect to
    ///
    /// # Returns
    ///
    /// * `Ok(WasmWebSocket)` - A connected WebSocket ready for use
    /// * `Err(NetworkError)` - If connection fails or is rejected
    ///
    /// # Connection Process
    ///
    /// 1. Creates a browser WebSocket instance
    /// 2. Sets up event handlers for open, message, error, and close events
    /// 3. Waits for either the 'open' event (success) or 'error' event (failure)
    /// 4. Returns the connected WebSocket or an error
    ///
    /// # Example
    ///
    /// ```no_run
    /// let ws = WasmWebSocket::connect("ws://localhost:8080").await?;
    /// ```
    pub async fn connect(url: &str) -> Result<Self, NetworkError> {
        let ws = WebSocket::new(url).map_err(|_| {
            NetworkError(format!("error creating websocket connection").to_string())
        })?;

        ws.set_binary_type(web_sys::BinaryType::Arraybuffer);

        // Create channels for messages
        let (sender, receiver) = mpsc::unbounded();
        let sender_rc = Rc::new(RefCell::new(sender));

        // Create a oneshot channel to signal connection status
        let (conn_tx, conn_rx) = oneshot::channel::<Result<(), NetworkError>>();
        let conn_tx = Rc::new(RefCell::new(Some(conn_tx)));

        // Set up open handler to signal successful connection
        let conn_tx_open = Rc::clone(&conn_tx);
        let on_open = Closure::wrap(Box::new(move |_: web_sys::Event| {
            if let Some(tx) = conn_tx_open.borrow_mut().take() {
                let _ = tx.send(Ok(()));
            }
        }) as Box<dyn FnMut(web_sys::Event)>);
        ws.set_onopen(Some(on_open.as_ref().unchecked_ref()));

        // Set up error handler to signal connection failure
        let conn_tx_error = Rc::clone(&conn_tx);
        let on_error = Closure::wrap(Box::new(move |e: web_sys::ErrorEvent| {
            let error_msg = if e.message().is_empty() {
                "WebSocket connection error".to_string()
            } else {
                e.message()
            };

            if let Some(tx) = conn_tx_error.borrow_mut().take() {
                let _ = tx.send(Err(NetworkError(format!(
                    "WebSocket connection failed: {}",
                    error_msg
                ))));
            }
        }) as Box<dyn FnMut(ErrorEvent)>);
        ws.set_onerror(Some(on_error.as_ref().unchecked_ref()));

        // Set up message handler
        let sender_for_msg = Rc::clone(&sender_rc);
        let on_message = Closure::wrap(Box::new(move |e: MessageEvent| {
            if let Ok(array_buffer) = e.data().dyn_into::<js_sys::ArrayBuffer>() {
                let array = Uint8Array::new(&array_buffer);
                let bytes = array.to_vec();

                if let Ok(sender) = sender_for_msg.try_borrow() {
                    let _ = sender.unbounded_send(WsMessage::Binary(bytes));
                }
            }
        }) as Box<dyn FnMut(MessageEvent)>);
        ws.set_onmessage(Some(on_message.as_ref().unchecked_ref()));

        // Set up close handler
        let sender_for_close = Rc::clone(&sender_rc);
        let on_close = Closure::wrap(Box::new(move |_| {
            if let Ok(sender) = sender_for_close.try_borrow() {
                let _ = sender.unbounded_send(WsMessage::Close);
            }
        }) as Box<dyn FnMut(CloseEvent)>);
        ws.set_onclose(Some(on_close.as_ref().unchecked_ref()));

        // Wait for connection to complete
        let connection_result = conn_rx
            .await
            .map_err(|_| NetworkError(format!("connection attempt was cancelled").to_string()))?;

        connection_result?;

        let websocket = Self {
            ws,
            _closures: ClosureHandlers {
                _on_message: on_message,
                _on_close: on_close,
                _on_error: on_error,
                _on_open: on_open,
            },
            receiver,
        };

        Ok(websocket)
    }

    /// Send binary data through the WebSocket
    ///
    /// # Arguments
    ///
    /// * `data` - The binary data to send
    ///
    /// # Returns
    ///
    /// * `Ok(())` - Data was successfully queued for sending
    /// * `Err(NetworkError)` - If the WebSocket is not in a state to send data
    ///
    /// # Note
    ///
    /// This method queues data for sending but doesn't guarantee delivery.
    /// The actual transmission happens asynchronously.
    pub fn send(&self, data: Vec<u8>) -> Result<(), NetworkError> {
        let array = Uint8Array::from(&data[..]);
        self.ws
            .send_with_array_buffer(&array.buffer())
            .map_err(|e| NetworkError(format!("failed to send message: {:?}", e)))?;
        Ok(())
    }

    /// Close the WebSocket connection
    ///
    /// This initiates a graceful close of the WebSocket connection.
    /// Any pending messages may still be delivered before the connection fully closes.
    ///
    /// # Returns
    ///
    /// * `Ok(())` - Close was initiated successfully
    /// * `Err(NetworkError)` - If the WebSocket is already closed or in an invalid state
    pub fn close(&self) -> Result<(), NetworkError> {
        self.ws
            .close()
            .map_err(|_| NetworkError("Failed to close WebSocket".to_string()))?;
        Ok(())
    }
}

// Safe because WASM in single-threaded
#[cfg(feature = "wasm")]
unsafe impl Send for WasmWebSocket {}

/// Receives messages from the WebSocket as a Stream
///
/// # Message Handling
///
/// - Binary messages: Passed through as-is
/// - Text messages: Converted to errors (protocol violation)
/// - Close messages: Ends the stream
/// - Ping/Pong: Handled internally, not exposed to consumers
#[cfg(feature = "wasm")]
impl Stream for WasmWebSocket {
    type Item = Result<Vec<u8>, NetworkError>;

    fn poll_next(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Option<Self::Item>> {
        match Pin::new(&mut self.receiver).poll_next(cx) {
            Poll::Ready(Some(WsMessage::Binary(data))) => Poll::Ready(Some(Ok(data))),
            Poll::Ready(Some(WsMessage::Text(_))) => Poll::Ready(Some(Err(NetworkError(
                "unexpected text message on websocket".to_string(),
            )))),
            Poll::Ready(Some(WsMessage::Close)) => Poll::Ready(None),
            Poll::Ready(Some(WsMessage::Ping(_)) | Some(WsMessage::Pong(_))) => {
                // Skip ping/pong messages and poll again
                cx.waker().wake_by_ref();
                Poll::Pending
            }
            Poll::Ready(None) => Poll::Ready(None),
            Poll::Pending => Poll::Pending,
        }
    }
}

/// Sends messages to the WebSocket as a Sink
///
/// This implementation:
/// - Accepts `Vec<u8>` binary data
/// - Is always ready to accept messages (buffering handled by browser)
/// - Sends data immediately without internal buffering
/// - Gracefully closes the connection when the sink is closed
///
/// # Backpressure
///
/// The browser's WebSocket implementation handles buffering and backpressure.
/// This sink reports as always ready, relying on the browser's internal queue.
#[cfg(feature = "wasm")]
impl Sink<Vec<u8>> for WasmWebSocket {
    type Error = NetworkError;

    fn poll_ready(
        self: Pin<&mut Self>,
        _cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }

    fn start_send(self: Pin<&mut Self>, item: Vec<u8>) -> Result<(), Self::Error> {
        self.as_ref().send(item)
    }

    fn poll_flush(
        self: Pin<&mut Self>,
        _cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }

    fn poll_close(
        self: Pin<&mut Self>,
        _cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Result<(), Self::Error>> {
        let _ = self.as_ref().close();
        Poll::Ready(Ok(()))
    }
}
