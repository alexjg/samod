use crate::{ConnFinishedReason, ConnectionId, PeerInfo};

mod connection_handle;
pub(crate) use connection_handle::ConnectionHandle;

/// A connection to some remote
///
/// This is returned by [`Repo::connect`] and friends and is a kind of handle
/// which can be used to learn about events on the connection
///
/// Use [`Connection::handshake_complete`] to wait for the handshake to complete
/// and find out the remote peer ID.
///
/// Use [`Connection::finished`] to wait for the connection to finish and find
/// out why the connection finished.
#[derive(Clone)]
pub struct Connection {
    handle: ConnectionHandle,
}

impl std::fmt::Debug for Connection {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Connection")
            .field("id", &self.handle.id())
            .finish()
    }
}

impl Connection {
    pub(crate) fn new(handle: ConnectionHandle) -> Self {
        Self { handle }
    }

    /// Returns the ID of this connection. This ID is unique within a given
    /// [`Repo`]
    pub fn id(&self) -> ConnectionId {
        self.handle.id()
    }

    /// Returns the [`PeerInfo`] of the remote peer if the handshake has
    /// completed, `None` otherwise.
    pub fn info(&self) -> Option<PeerInfo> {
        self.handle.info()
    }

    /// Wait for the handshake to complete and return the [`PeerInfo`] of the
    /// remote peer.
    ///
    /// If the handshake has already completed, or the connection is already
    /// finished, the returned future will resolve immediately.
    ///
    /// If the connection terminates whilst waiting for the handshake the future
    /// will resolve with an `Err`.
    pub fn handshake_complete(
        &self,
    ) -> impl Future<Output = Result<PeerInfo, ConnFinishedReason>> + 'static {
        self.handle.handshake_complete()
    }

    /// Wait for the connection to finish and return the reason why the
    /// connection finished. If the connection has already finished, the returned
    /// future will resolve immediately.
    pub fn finished(&self) -> impl Future<Output = ConnFinishedReason> + Sync + Send + 'static {
        self.handle.finished()
    }
}
