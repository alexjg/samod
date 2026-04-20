use std::collections::HashMap;

use crate::ConnectionId;

/// The state of a search for a document
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DocSearch {
    pub(crate) phase: DocSearchPhase,
    pub(crate) pending_connections: Vec<url::Url>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DocSearchPhase {
    Loading,
    Searching(HashMap<ConnectionId, PeerRequestState>),
    Ready,
}

impl DocSearch {
    pub fn phase(&self) -> &DocSearchPhase {
        &self.phase
    }

    pub fn pending_connections(&self) -> &[url::Url] {
        &self.pending_connections
    }

    pub fn is_currently_unavailable(&self) -> bool {
        let DocSearchPhase::Searching(peers) = &self.phase else {
            return false;
        };
        if !self.pending_connections.is_empty() {
            return false;
        }
        peers
            .values()
            .all(|p| matches!(p, PeerRequestState::Unavailable))
    }
}

/// State of searching with a specific peer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PeerRequestState {
    /// We've asked this peer and are waiting for a response.
    Requested,
    /// This peer doesn't have the document.
    Unavailable,
    /// We're syncing the document from this peer.
    Syncing,
    /// The peer has the document and we are in sync (Ready phase, idle).
    Available,
}
