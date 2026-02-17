use std::sync::{Arc, Mutex};

use futures::channel::oneshot;
use samod_core::DialerId;

use crate::{ConnectionId, PeerInfo, Repo, unbounded};

/// Lifecycle events for a dialer
#[derive(Debug, Clone)]
pub enum DialerEvent {
    /// A connection was established and the handshake completed.
    Connected {
        /// Information about the connected peer.
        peer_info: PeerInfo,
    },
    /// The active connection was lost. The dialer will attempt to
    /// reconnect according to the backoff configuration.
    Disconnected {
        /// A description of why the connection was lost.
        reason: String,
    },
    /// A reconnection attempt is starting after backoff.
    Reconnecting {
        /// The current attempt number (1-based).
        attempt: u32,
    },
    /// The dialer has permanently failed (max retries exceeded).
    /// No further reconnection attempts will be made.
    MaxRetriesReached,
}

/// Error returned when a dialer permanently fails before establishing a
/// connection.
#[derive(Debug, Clone)]
pub struct DialerFailed;

impl std::fmt::Display for DialerFailed {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "dialer permanently failed (max retries exceeded)")
    }
}

impl std::error::Error for DialerFailed {}

/// Handle to a dialer with automatic reconnection.
///
/// Returned by [`Repo::dial()`]. Represents a dialer that automatically
/// reconnects with backoff when the connection is lost.
///
/// The handle can be used to:
/// - Wait for the first connection with [`DialerHandle::established()`]
/// - Observe lifecycle events with [`DialerHandle::events()`]
/// - Check connection status with [`DialerHandle::is_connected()`]
/// - Shut down the dialer with [`DialerHandle::close()`]
#[derive(Clone)]
pub struct DialerHandle {
    inner: Arc<Mutex<DialerHandleInner>>,
    dialer_id: DialerId,
    repo: Repo,
}

struct DialerHandleInner {
    /// Current peer info if connected.
    peer_info: Option<PeerInfo>,
    /// Current connection ID if connected.
    connection_id: Option<ConnectionId>,
    /// Whether the dialer has permanently failed.
    permanently_failed: bool,
    /// Senders for event subscribers.
    event_senders: Vec<unbounded::UnboundedSender<DialerEvent>>,
    /// Waiters for the `established()` future.
    established_waiters: Vec<oneshot::Sender<Result<PeerInfo, DialerFailed>>>,
}

impl DialerHandle {
    pub(crate) fn new(dialer_id: DialerId, repo: Repo) -> Self {
        Self {
            inner: Arc::new(Mutex::new(DialerHandleInner {
                peer_info: None,
                connection_id: None,
                permanently_failed: false,
                event_senders: Vec::new(),
                established_waiters: Vec::new(),
            })),
            dialer_id,
            repo,
        }
    }

    /// The dialer ID for this dialer.
    pub fn id(&self) -> DialerId {
        self.dialer_id
    }

    /// Wait for the dialer to establish a connection and complete the
    /// handshake. Resolves immediately if already connected.
    ///
    /// Returns `Err` if the dialer permanently fails before
    /// establishing a connection (e.g. max retries exceeded).
    pub fn established(&self) -> impl Future<Output = Result<PeerInfo, DialerFailed>> + 'static {
        let immediate_result;
        let rx;

        {
            let mut inner = self.inner.lock().unwrap();
            // If already connected, return immediately
            if let Some(ref peer_info) = inner.peer_info {
                immediate_result = Some(Ok(peer_info.clone()));
                rx = None;
            } else if inner.permanently_failed {
                // If permanently failed, return immediately
                immediate_result = Some(Err(DialerFailed));
                rx = None;
            } else {
                let (tx, channel_rx) = oneshot::channel();
                inner.established_waiters.push(tx);
                immediate_result = None;
                rx = Some(channel_rx);
            }
        }

        async move {
            if let Some(result) = immediate_result {
                return result;
            }
            rx.unwrap().await.unwrap_or(Err(DialerFailed))
        }
    }

    /// Returns a stream of lifecycle events for this dialer.
    ///
    /// The stream yields events for connect, disconnect, reconnect
    /// attempts, and permanent failure. Useful for metrics, logging,
    /// and application-level health monitoring.
    pub fn events(&self) -> impl futures::Stream<Item = DialerEvent> + Unpin {
        let (tx, rx) = unbounded::channel();
        let mut inner = self.inner.lock().unwrap();
        inner.event_senders.push(tx);
        rx
    }

    /// Returns the peer info if currently connected, `None` otherwise.
    pub fn peer_info(&self) -> Option<PeerInfo> {
        self.inner.lock().unwrap().peer_info.clone()
    }

    /// Returns true if the dialer currently has an active connection.
    pub fn is_connected(&self) -> bool {
        self.inner.lock().unwrap().peer_info.is_some()
    }

    /// Returns the connection ID of the active connection, if any.
    ///
    /// This can be used with [`DocHandle::we_have_their_changes`](crate::DocHandle::we_have_their_changes)
    /// to wait until a document has been fully synced with the dialed peer.
    pub fn connection_id(&self) -> Option<ConnectionId> {
        self.inner.lock().unwrap().connection_id
    }

    /// Shut down this dialer. Closes the active connection (if any)
    /// and stops reconnection attempts.
    pub fn close(&self) {
        // Remove the dialer from the repo, which will close any active
        // connections and stop reconnection.
        let _ = self.repo.remove_dialer_by_id(self.dialer_id);
    }

    // -- Internal methods called from Inner::handle_event --

    /// Notify the handle that a connection was established.
    pub(crate) fn notify_connected(&self, peer_info: PeerInfo, connection_id: ConnectionId) {
        let mut inner = self.inner.lock().unwrap();
        inner.peer_info = Some(peer_info.clone());
        inner.connection_id = Some(connection_id);

        // Notify established waiters
        for waiter in inner.established_waiters.drain(..) {
            let _ = waiter.send(Ok(peer_info.clone()));
        }

        // Broadcast event
        let event = DialerEvent::Connected {
            peer_info: peer_info.clone(),
        };
        inner
            .event_senders
            .retain(|tx| tx.unbounded_send(event.clone()).is_ok());
    }

    /// Notify the handle that the connection was lost.
    pub(crate) fn notify_disconnected(&self, reason: String) {
        let mut inner = self.inner.lock().unwrap();
        inner.peer_info = None;
        inner.connection_id = None;

        let event = DialerEvent::Disconnected { reason };
        inner
            .event_senders
            .retain(|tx| tx.unbounded_send(event.clone()).is_ok());
    }

    /// Notify the handle that a reconnection attempt is starting.
    pub(crate) fn notify_reconnecting(&self, attempt: u32) {
        let mut inner = self.inner.lock().unwrap();

        let event = DialerEvent::Reconnecting { attempt };
        inner
            .event_senders
            .retain(|tx| tx.unbounded_send(event.clone()).is_ok());
    }

    /// Notify the handle that max retries have been reached.
    pub(crate) fn notify_max_retries_reached(&self) {
        let mut inner = self.inner.lock().unwrap();
        inner.permanently_failed = true;

        // Notify established waiters of failure
        for waiter in inner.established_waiters.drain(..) {
            let _ = waiter.send(Err(DialerFailed));
        }

        let event = DialerEvent::MaxRetriesReached;
        inner
            .event_senders
            .retain(|tx| tx.unbounded_send(event.clone()).is_ok());
    }
}

impl std::fmt::Debug for DialerHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DialerHandle")
            .field("dialer_id", &self.dialer_id)
            .finish()
    }
}
