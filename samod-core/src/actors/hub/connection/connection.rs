use crate::{
    ConnectionId, DocumentId, PeerId, UnixTimestamp,
    actors::hub::{HubResults, connection::repo_sync_connection::RepoSyncConnection},
    network::{
        ConnDirection, ConnectionInfo, ConnectionOwner, ConnectionProtocol,
        PeerDocState, PeerMetadata,
    },
};

#[cfg(feature = "subduction")]
use crate::{
    actors::hub::connection::SubductionConnection,
    network::ConnectionState,
};

use super::{ConnOut, EstablishedConnection, ReceiveEvent};

/// State of a network connection throughout its lifecycle.
#[derive(Debug, Clone)]
pub struct Connection {
    id: ConnectionId,
    /// The dialer or listener that owns this connection.
    owner: ConnectionOwner,
    local_peer_id: PeerId,
    local_metadata: Option<PeerMetadata>,
    /// When the connection was created
    #[allow(dead_code)]
    created_at: UnixTimestamp,
    last_received: Option<UnixTimestamp>,
    last_sent: Option<UnixTimestamp>,
    // Whether our state has changed since we last notified the event loop
    // (via `pop_new_state`)
    dirty: bool,
    conn_type: ConnType,
}

#[derive(Debug, Clone)]
enum ConnType {
    Repo(RepoSyncConnection),
    #[cfg(feature = "subduction")]
    Subduction(SubductionConnection),
}

/// The phase of a connection's lifecycle.
#[derive(Debug, Clone)]
pub(crate) enum ConnectionPhase {
    WaitingForPeer,
    WaitingForJoin,
    Established(EstablishedConnection),
    Closed,
}

pub(crate) struct ConnectionArgs {
    pub(crate) direction: ConnDirection,
    pub(crate) owner: ConnectionOwner,
    pub(crate) local_peer_id: PeerId,
    pub(crate) local_metadata: Option<PeerMetadata>,
    pub(crate) created_at: UnixTimestamp,
}

impl Connection {
    /// Create a new connection in handshaking state.
    pub(crate) fn new(
        out: &mut HubResults,
        protocol: ConnectionProtocol,
        args: ConnectionArgs,
    ) -> Self {
        let id = ConnectionId::new();
        let ConnectionArgs {
            direction: _,
            ref owner,
            ref local_peer_id,
            ref local_metadata,
            ref created_at,
        } = args;
        let local_peer_id = local_peer_id.clone();
        let local_metadata = local_metadata.clone();
        let created_at = *created_at;
        let owner = owner.clone();
        let mut conn_out = ConnOut::new(id, created_at, out);
        let conn_type = match protocol {
            ConnectionProtocol::AutomergeSync => {
                ConnType::Repo(RepoSyncConnection::new_handshaking(&mut conn_out, id, args))
            }
            #[cfg(feature = "subduction")]
            ConnectionProtocol::Subduction { .. } => {
                ConnType::Subduction(SubductionConnection::new(id, owner.clone(), created_at))
            }
        };
        let last_sent = conn_out.last_sent();
        Self {
            id,
            owner,
            local_peer_id,
            local_metadata,
            created_at,
            last_received: None,
            last_sent,
            dirty: true,
            conn_type,
        }
    }

    pub(crate) fn id(&self) -> ConnectionId {
        self.id
    }

    pub(crate) fn owner(&self) -> ConnectionOwner {
        self.owner
    }

    pub(crate) fn receive_msg(
        &mut self,
        out: &mut HubResults,
        now: UnixTimestamp,
        msg: Vec<u8>,
    ) -> Result<Vec<ReceiveEvent>, ReceiveError> {
        self.dirty = true;
        self.last_received = Some(now);
        match &mut self.conn_type {
            ConnType::Repo(repo_conn) => {
                let mut conn_out = ConnOut::new(self.id, now, out);
                let result = repo_conn.receive_msg(&mut conn_out, now, msg);
                if let Some(last_sent) = conn_out.last_sent() {
                    self.last_sent = Some(last_sent);
                }
                result
            }
            #[cfg(feature = "subduction")]
            ConnType::Subduction(_subduction_conn) => {
                Ok(vec![ReceiveEvent::SubductionMessage(msg)])
            }
        }
    }

    /// Get the established connection if in established phase.
    pub(crate) fn established_connection(&self) -> Option<&EstablishedConnection> {
        let ConnType::Repo(repo_conn) = &self.conn_type else {
            return None;
        };
        repo_conn.established_connection()
    }

    /// Get the established connection if in established phase.
    pub(crate) fn established_connection_mut(&mut self) -> Option<&mut EstablishedConnection> {
        self.dirty = true;
        let ConnType::Repo(repo_conn) = &mut self.conn_type else {
            return None;
        };
        repo_conn.established_connection_mut()
    }

    /// Get the remote peer ID if established.
    pub(crate) fn remote_peer_id(&self) -> Option<&PeerId> {
        let ConnType::Repo(repo_conn) = &self.conn_type else {
            return None;
        };
        repo_conn.remote_peer_id()
    }

    /// Add a document subscription.
    pub(crate) fn add_document(&mut self, document_id: DocumentId) {
        self.dirty = true;
        let ConnType::Repo(repo_conn) = &mut self.conn_type else {
            panic!("Cannot add document subscription in non-repo connection");
        };
        repo_conn.add_document(document_id);
    }

    pub(crate) fn update_peer_state(&mut self, document_id: &DocumentId, state: PeerDocState) {
        self.dirty = true;
        let ConnType::Repo(repo_conn) = &mut self.conn_type else {
            return;
        };
        repo_conn.update_peer_state(document_id, state);
    }

    pub(crate) fn is_closed(&self) -> bool {
        let ConnType::Repo(repo_conn) = &self.conn_type else {
            return false;
        };
        return repo_conn.is_closed();
    }

    /// Send a message on this connection, automatically tracking the sent timestamp.
    pub(crate) fn send(&mut self, out: &mut HubResults, now: UnixTimestamp, msg: Vec<u8>) {
        let mut conn_out = ConnOut::new(self.id, now, out);
        conn_out.send(self.remote_peer_id(), msg);
        self.dirty = true;
        self.last_sent = Some(now);
    }

    pub(crate) fn last_received(&self) -> Option<UnixTimestamp> {
        self.last_received
    }

    pub(crate) fn last_sent(&self) -> Option<UnixTimestamp> {
        self.last_sent
    }

    pub(crate) fn info(&self) -> ConnectionInfo {
        match &self.conn_type {
            ConnType::Repo(repo_conn) => {
                let repo_info = repo_conn.info();
                ConnectionInfo {
                    id: self.id,
                    last_received: self.last_received,
                    last_sent: self.last_sent,
                    docs: repo_info.docs,
                    state: repo_info.state,
                }
            }
            #[cfg(feature = "subduction")]
            ConnType::Subduction(_) => ConnectionInfo {
                id: self.id,
                last_received: self.last_received,
                last_sent: self.last_sent,
                docs: std::collections::HashMap::new(),
                state: ConnectionState::Handshaking,
            },
        }
    }

    // If this connection state has changed since the last call to
    // `pop_new_info`, return the new info
    pub(crate) fn pop_new_info(&mut self) -> Option<ConnectionInfo> {
        if self.dirty {
            self.dirty = false;
            Some(self.info())
        } else {
            None
        }
    }
}

#[derive(Debug)]
pub(crate) struct ReceiveError(String);

impl ReceiveError {
    pub(crate) fn new(msg: impl Into<String>) -> Self {
        Self(msg.into())
    }
}

impl std::fmt::Display for ReceiveError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl std::error::Error for ReceiveError {}
