use url::Url;

use super::DialerId;

/// A request for the IO layer to establish a new transport for a dialer.
///
/// When a dialer needs a connection (either on initial registration or
/// after a reconnection backoff expires), the hub emits a `DialRequest`.
/// The IO layer should:
///
/// 1. Attempt to establish a transport to the given URL.
/// 2. On success: call `HubEvent::create_dialer_connection(dialer_id)` to get
///    a `ConnectionId`, wire up the stream/sink, then start driving the connection.
/// 3. On failure: call `HubEvent::dial_failed(dialer_id, error)`.
#[derive(Debug, Clone)]
pub struct DialRequest {
    /// The dialer that needs a transport.
    pub dialer_id: DialerId,
    /// The URL to connect to.
    pub url: Url,
}
