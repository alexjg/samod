use futures::{Future, Sink, SinkExt, Stream, StreamExt};

use crate::{ConnDirection, ConnFinishedReason, Samod};

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

impl Samod {
    /// Connect a websocket
    ///
    /// This function waits until the initial handshake is complete and then returns a future which
    /// must be driven to completion to keep the connection alive. The returned future is required
    /// so that we continue to respond to pings from the server which will otherwise disconnect us.
    ///
    /// ## Example
    ///
    /// ```no_run
    /// use automerge_repo::{Repo, RepoHandle, ConnDirection, Storage};
    ///
    /// #[tokio::main]
    /// async fn main() {
    ///    let storage: Box<dyn Storage> = unimplemented!();
    ///    let repo = Repo::new(None, storage);
    ///    let handle = repo.run();
    ///    let (conn, _) = tokio_tungstenite::connect_async("ws://localhost:8080").await.unwrap();
    ///    let conn_driver = handle.connect_tungstenite(conn, ConnDirection::Outgoing).await.unwrap();
    ///    tokio::spawn(async move {
    ///        let finished_reason = conn_driver.await;
    ///        eprintln!("Repo connection finished: {}", finished_reason);
    ///    });
    ///    // ...
    /// }
    /// ```
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
    ///
    /// This function waits until the initial handshake is complete and then returns a future which
    /// must be driven to completion to keep the connection alive.
    ///
    /// ## Example
    ///
    /// ```no_run
    /// use automerge_repo::{Repo, RepoHandle, ConnDirection, Storage};
    ///
    /// #[tokio::main]
    /// async fn main() {
    ///     let storage: Box<dyn Storage> = unimplemented!();
    ///     let handle = Repo::new(None, storage).run();
    ///     let app = axum::Router::new()
    ///         .route("/", axum::routing::get(websocket_handler))
    ///         .with_state(handle.clone());
    ///     let server = axum::Server::bind(&"0.0.0.0:0".parse().unwrap()).serve(app.into_make_service());
    ///     let port = server.local_addr().port();
    ///     tokio::spawn(server);
    /// }
    ///
    /// async fn websocket_handler(
    ///     ws: axum::extract::ws::WebSocketUpgrade,
    ///     axum::extract::State(handle): axum::extract::State<RepoHandle>,
    /// ) -> axum::response::Response {
    ///     ws.on_upgrade(|socket| handle_socket(socket, handle))
    /// }
    ///
    /// async fn handle_socket(
    ///     socket: axum::extract::ws::WebSocket,
    ///     repo: RepoHandle,
    /// ) {
    ///     let driver = repo
    ///         .accept_axum(socket)
    ///         .await
    ///         .unwrap();
    ///     tokio::spawn(async {
    ///         let finished_reason = driver.await;
    ///         eprintln!("Repo connection finished: {}", finished_reason);
    ///     });
    /// }
    /// ```
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

    pub fn connect_websocket<S, M>(
        &self,
        stream: S,
        direction: ConnDirection,
    ) -> impl Future<Output = ConnFinishedReason> + 'static
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
            .boxed();

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
