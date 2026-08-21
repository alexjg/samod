use std::error::Error;

use futures::future::BoxFuture;
use url::Url;

use crate::Transport;

/// Knows how to establish a transport to a remote endpoint.
///
/// Implementations provide both *where* to connect ([`Dialer::url`]) and
/// *how* to connect ([`Dialer::connect`]). A single `Dialer` instance
/// can be shared across multiple connectors (via `Arc`).
pub trait Dialer: Send + Sync + 'static {
    /// The error produced when the dialer permanently fails.
    ///
    /// Implementations which never produce permanent failures can use
    /// [`std::convert::Infallible`].
    type Error: Error + Send + Sync + 'static;

    /// The URL identifying the remote endpoint.
    ///
    /// This is used for logging and debugging
    fn url(&self) -> Url;

    /// Establish a new transport to the remote endpoint.
    ///
    /// Called each time the dialer needs a connection — both on the
    /// initial dial and on each reconnection attempt after backoff.
    /// Return [`DialError::TransientFailure`] to retry, or
    /// [`DialError::PermanentFailure`] to stop without retrying.
    fn connect(&self) -> BoxFuture<'static, Result<Transport, DialError<Self::Error>>>;
}

/// An error returned by [`Dialer::connect`].
///
/// Transient failures are retried according to the dialer's backoff
/// configuration. Permanent failures stop the dialer immediately and are
/// returned by [`DialerHandle::established`](crate::DialerHandle::established).
#[derive(Debug)]
pub enum DialError<E> {
    /// The dialer cannot recover from this error by retrying.
    PermanentFailure(E),
    /// Establishing the transport failed, but a later attempt may succeed.
    TransientFailure(Box<dyn Error + Send + Sync + 'static>),
}

impl<E> DialError<E> {
    /// Construct a transient failure from any thread-safe error.
    pub fn transient(error: impl Error + Send + Sync + 'static) -> Self {
        Self::TransientFailure(Box::new(error))
    }
}

impl<E: std::fmt::Display> std::fmt::Display for DialError<E> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::PermanentFailure(error) => write!(f, "permanent dial failure: {error}"),
            Self::TransientFailure(error) => write!(f, "transient dial failure: {error}"),
        }
    }
}

impl<E: Error + 'static> Error for DialError<E> {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::PermanentFailure(error) => Some(error),
            Self::TransientFailure(error) => Some(error.as_ref()),
        }
    }
}
