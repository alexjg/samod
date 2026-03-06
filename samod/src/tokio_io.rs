use std::sync::Arc;

use futures::FutureExt;
use url::Url;

use crate::{Dialer, Repo, Transport};

/// A [`Dialer`] that connects to a TCP endpoint
///
/// The dialer is constructed via a [Url]. The host will be resolved using
/// [`tokio::net::TcpStream::connect`] on [`Url::host_str()`] and [`Url::port()`]. This is the same
/// as calling [`tokio::net::lookup_host`] directly.
/// Any more advanced resolution should be performed with a custom [`Dialer`] implementation.
///
/// The dialer will use a simple length delimited framing to connect to the
/// other end. Accepting connections on the other end should be done using
/// [`Transport::from_tokio_io`].
///
/// # Example
///
/// ```rust
/// use std::sync::Arc;
/// use samod::{Repo, Transport, BackoffConfig};
/// use samod::tokio_io::TcpDialer;
/// use tokio::net::TcpListener;
///
/// # async fn example() {
///
/// // First start a server
/// let listener = TcpListener::bind("0.0.0.0:0").await.unwrap();
/// tokio::spawn(async move {
///     let repo: Repo = Repo::build_tokio().load().await;
///     let acceptor = repo.make_acceptor(url::Url::parse("tcp://someserver").unwrap()).unwrap();
///     let (io, _) = listener.accept().await.unwrap();
///     acceptor.accept(Transport::from_tokio_io(io));
/// });
///
/// // Now make a client which dials the server
/// let repo: Repo = Repo::build_tokio().load().await;
/// let dialer = TcpDialer::new(url::Url::parse("tcp://0.0.0.0:0").unwrap()).unwrap();
/// repo.dial(BackoffConfig::default(), Arc::new(dialer)).unwrap();
/// # }
/// ```
pub struct TcpDialer {
    host: String,
    port: u16,
}

pub enum TcpDialerError {
    InvalidUrl(String),
    RepoStopped,
}

impl std::fmt::Display for TcpDialerError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match &self {
            TcpDialerError::InvalidUrl(message) => {
                write!(f, "Error creating TcpDialer: {}", message)
            }
            TcpDialerError::RepoStopped => write!(f, "{}", crate::Stopped {}),
        }
    }
}

impl std::fmt::Debug for TcpDialerError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{self}")
    }
}

impl std::error::Error for TcpDialerError {}

impl TcpDialer {
    /// Create a dialer which will resolve the given [`Url`].
    /// The [`Url`] must have the scheme `tcp://`, and must have a valid [`Url::host`] and [`Url::port`].
    pub fn new(url: Url) -> Result<Self, TcpDialerError> {
        if url.scheme() != "tcp" {
            return Err(TcpDialerError::InvalidUrl(format!(
                "Provided URL {url} is not of scheme tcp://!"
            )));
        };

        let Some(host) = url.host_str() else {
            return Err(TcpDialerError::InvalidUrl(format!(
                "No host provided for {url}!"
            )));
        };

        let Some(port) = url.port() else {
            return Err(TcpDialerError::InvalidUrl(format!(
                "No port provided for {url}!"
            )));
        };

        Ok(Self {
            host: host.to_string(),
            port,
        })
    }
}

impl Dialer for TcpDialer {
    fn url(&self) -> url::Url {
        Url::parse(&format!("tcp://{}:{}", self.host, self.port)).unwrap()
    }

    fn connect(
        &self,
    ) -> futures::future::BoxFuture<
        'static,
        Result<crate::Transport, Box<dyn std::error::Error + Send + Sync + 'static>>,
    > {
        let host = self.host.clone();
        let port = self.port;
        async move {
            let io = tokio::net::TcpStream::connect((host, port)).await?;
            let transport = Transport::from_tokio_io(io);
            Ok(transport)
        }
        .boxed()
    }
}

impl Repo {
    /// Dial a TCP endpoint with automatic reconnection.
    ///
    /// Uses a built-in TCP [`Dialer`](crate::Dialer) that connects
    /// via `tokio-io`. The connection will automatically reconnect
    /// with the provided backoff configuration when the connection is lost.
    ///
    /// This is a convenience wrapper around [`Repo::dial`] with a
    /// [`TcpDialer`].
    ///
    /// # Arguments
    ///
    /// * `url` - The TCP URL to connect to (e.g. `"tcp://sync.example.com"`).
    ///   It must have the scheme `tcp://`, and must have a valid [`Url::host`] and [`Url::port`].
    /// * `backoff` - Backoff configuration for reconnection attempts.
    ///
    /// # Returns
    ///
    /// A [`DialerHandle`](crate::DialerHandle) for observing and controlling the dialer.
    pub fn dial_tcp(
        &self,
        url: Url,
        backoff: crate::BackoffConfig,
    ) -> Result<crate::DialerHandle, TcpDialerError> {
        let dialer = Arc::new(TcpDialer::new(url)?);
        self.dial(backoff, dialer)
            .map_err(|_| TcpDialerError::RepoStopped)
    }
}
