use std::{
    collections::HashSet,
    sync::{Arc, Mutex},
};

use crate::{ConnectionId, connection::ConnectionHandle};
use samod_core::ListenerId;

use crate::{ConnFinishedReason, PeerInfo, Repo, Stopped, unbounded};

/// Lifecycle events for an acceptor.
#[derive(Debug, Clone)]
pub enum AcceptorEvent {
    /// A new inbound connection completed its handshake.
    ClientConnected {
        /// Information about the connected peer.
        peer_info: PeerInfo,
        /// The connection ID for correlation.
        connection_id: ConnectionId,
    },
    /// An inbound connection was lost.
    ClientDisconnected {
        /// The connection ID that was lost.
        connection_id: ConnectionId,
        /// The reason the connection was lost.
        reason: ConnFinishedReason,
    },
}

/// Handle to an acceptor (listener) endpoint that accepts inbound connections.
///
/// Returned by [`Repo::make_acceptor()`]. Represents a listener endpoint that
/// accepts inbound connections. The handle is used to feed individual
/// accepted connections via [`AcceptorHandle::accept()`].
///
/// The handle can be used to:
/// - Accept inbound connections with [`AcceptorHandle::accept()`]
/// - Observe lifecycle events with [`AcceptorHandle::events()`]
/// - Check connection count with [`AcceptorHandle::connection_count()`]
/// - Shut down the acceptor with [`AcceptorHandle::close()`]
#[derive(Clone)]
pub struct AcceptorHandle {
    inner: Arc<Mutex<AcceptorHandleInner>>,
    listener_id: ListenerId,
    repo: Repo,
}

struct AcceptorHandleInner {
    /// Connections that have successfully completed their handshake.
    active_connections: HashSet<ConnectionId>,
    /// Senders for event subscribers.
    event_senders: Vec<unbounded::UnboundedSender<AcceptorEvent>>,
}

impl AcceptorHandle {
    pub(crate) fn new(listener_id: ListenerId, repo: Repo) -> Self {
        Self {
            inner: Arc::new(Mutex::new(AcceptorHandleInner {
                active_connections: HashSet::new(),
                event_senders: Vec::new(),
            })),
            listener_id,
            repo,
        }
    }

    /// The listener ID for this acceptor.
    pub fn id(&self) -> ListenerId {
        self.listener_id
    }

    /// Accept an inbound connection.
    ///
    /// Wires up the transport to the hub and starts driving the connection.
    /// This is typically called from a server framework's connection handler
    /// (e.g. an axum WebSocket upgrade handler). Use
    /// [`Transport::with_expected_peer_id`](crate::Transport::with_expected_peer_id)
    /// to bind the connection to an externally authenticated peer identity.
    pub fn accept(&self, transport: crate::Transport) -> Result<ConnectionHandle, Stopped> {
        self.repo.accept_on_listener(self.listener_id, transport)
    }

    /// Accept the length delimited framed connection created by
    /// [`TcpDialer`](crate::tokio_io::TcpDialer). See the documentation for
    /// that type for examples.
    #[cfg(feature = "tokio")]
    pub fn accept_tokio_io<S>(&self, io: S) -> Result<ConnectionHandle, Stopped>
    where
        S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Send + Unpin + 'static,
    {
        use crate::Transport;
        self.repo
            .accept_on_listener(self.listener_id, Transport::from_tokio_io(io))
    }

    /// Returns a stream of lifecycle events for this acceptor.
    ///
    /// The stream yields events for client connections and disconnections.
    /// Useful for metrics (e.g. Prometheus gauges for active connection count).
    pub fn events(&self) -> impl futures::Stream<Item = AcceptorEvent> + Unpin {
        let (tx, rx) = unbounded::channel();
        let mut inner = self.inner.lock().unwrap();
        inner.event_senders.push(tx);
        rx
    }

    /// Returns the number of currently active connections on this endpoint.
    pub fn connection_count(&self) -> usize {
        self.inner.lock().unwrap().active_connections.len()
    }

    /// Shut down this acceptor. Closes all active connections and stops
    /// accepting new ones.
    pub fn close(&self) {
        let _ = self.repo.remove_listener_by_id(self.listener_id);
    }

    // -- Internal methods called from Inner::handle_event --

    /// Notify the handle that a client connected.
    pub(crate) fn notify_client_connected(&self, peer_info: PeerInfo, connection_id: ConnectionId) {
        let mut inner = self.inner.lock().unwrap();
        inner.active_connections.insert(connection_id);

        let event = AcceptorEvent::ClientConnected {
            peer_info,
            connection_id,
        };
        inner
            .event_senders
            .retain(|tx| tx.unbounded_send(event.clone()).is_ok());
    }

    /// Notify the handle that a client disconnected.
    pub(crate) fn notify_client_disconnected(
        &self,
        connection_id: ConnectionId,
        reason: ConnFinishedReason,
    ) {
        let mut inner = self.inner.lock().unwrap();
        inner.active_connections.remove(&connection_id);

        let event = AcceptorEvent::ClientDisconnected {
            connection_id,
            reason,
        };
        inner
            .event_senders
            .retain(|tx| tx.unbounded_send(event.clone()).is_ok());
    }
}

impl std::fmt::Debug for AcceptorHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AcceptorHandle")
            .field("listener_id", &self.listener_id)
            .finish()
    }
}
