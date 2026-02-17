use futures::future::BoxFuture;
use url::Url;

use crate::Transport;

/// Knows how to establish a transport to a remote endpoint.
///
/// Implementations provide both *where* to connect ([`Dialer::url`]) and
/// *how* to connect ([`Dialer::connect`]). A single `Dialer` instance
/// can be shared across multiple connectors (via `Arc<dyn Dialer>`).
pub trait Dialer: Send + Sync + 'static {
    /// The URL identifying the remote endpoint.
    ///
    /// This is used for logging and debugging
    fn url(&self) -> Url;

    /// Establish a new transport to the remote endpoint.
    ///
    /// Called each time the dialer needs a connection — both on the
    /// initial dial and on each reconnection attempt after backoff.
    fn connect(
        &self,
    ) -> BoxFuture<'static, Result<Transport, Box<dyn std::error::Error + Send + Sync + 'static>>>;
}
