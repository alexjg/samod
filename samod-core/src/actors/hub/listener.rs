use std::collections::HashSet;

use url::Url;

use crate::{ConnectionId, ListenerId};

/// Internal state for a listener tracked by the hub.
///
/// A listener passively accepts inbound connections. It holds zero or
/// more active connections simultaneously.
#[derive(Debug, Clone)]
pub(crate) struct ListenerState {
    #[expect(dead_code)]
    pub(crate) listener_id: ListenerId,
    pub(crate) url: Url,
    /// All currently active connections on this endpoint.
    pub(crate) active_connections: HashSet<ConnectionId>,
}

impl ListenerState {
    pub(crate) fn new(listener_id: ListenerId, url: Url) -> Self {
        Self {
            listener_id,
            url,
            active_connections: HashSet::new(),
        }
    }

    /// Add a connection to this listener's active set.
    pub(crate) fn add_connection(&mut self, connection_id: ConnectionId) {
        self.active_connections.insert(connection_id);
    }

    /// Remove a connection from this listener's active set.
    pub(crate) fn remove_connection(&mut self, connection_id: &ConnectionId) {
        self.active_connections.remove(connection_id);
    }
}
