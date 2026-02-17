use std::time::Duration;

use url::Url;

/// Configuration for a new dialer.
///
/// A dialer actively establishes outgoing connections and automatically
/// reconnects with exponential backoff when a connection is lost.
#[derive(Debug, Clone)]
pub struct DialerConfig {
    /// The URL to connect to (e.g. "wss://sync.example.com/automerge").
    pub url: Url,
    /// Backoff configuration for reconnection attempts.
    pub backoff: BackoffConfig,
}

/// Configuration for a new listener.
///
/// A listener passively accepts inbound connections. It never initiates
/// connections and has no retry logic.
#[derive(Debug, Clone)]
pub struct ListenerConfig {
    /// URL identifying this listener endpoint (e.g. "ws://0.0.0.0:8080").
    /// Used for logging and identifying the endpoint in debugging output.
    pub url: Url,
}

/// Configuration for exponential backoff with jitter.
///
/// Used by dialers to control reconnection timing after a connection
/// is lost or a transport establishment fails.
///
/// The backoff formula is:
/// ```text
/// delay = min(initial_delay * 2^attempts, max_delay)
/// jittered_delay = delay * random(0.5, 1.0)
/// ```
#[derive(Debug, Clone)]
pub struct BackoffConfig {
    /// Delay before the first reconnection attempt.
    pub initial_delay: Duration,
    /// Maximum delay between reconnection attempts.
    pub max_delay: Duration,
    /// Maximum number of reconnection attempts before giving up.
    /// `None` means retry forever.
    pub max_retries: Option<u32>,
}

impl Default for BackoffConfig {
    fn default() -> Self {
        Self {
            initial_delay: Duration::from_millis(100),
            max_delay: Duration::from_secs(30),
            max_retries: None,
        }
    }
}
