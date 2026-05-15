use std::collections::HashMap;

pub use samod_core::PeerRequestState;
use samod_core::{ConnectionId, DocSearch, DocSearchPhase, DocumentActorId};

use crate::{DocHandle, actor_handle::ActorHandle};

/// The current state of a document search started by
/// [`Repo::search`](crate::Repo::search).
///
/// The search API emits a [`SearchState`] each time the state of the
/// ongoing search changes: peers are contacted, respond with whether they
/// have the document, sync progresses, or dialers connect or fail.
#[derive(Debug, Clone)]
pub enum SearchState {
    /// We're loading the document off of disk
    Loading,
    /// We're requesting the document from peers
    Requesting(Request),
    /// The document has been found and is available through the returned
    /// [`DocHandle`].
    Found(DocHandle),
}

impl SearchState {
    pub(crate) fn from_doc_search(
        search: DocSearch,
        actor_id: DocumentActorId,
        actors: &HashMap<DocumentActorId, ActorHandle>,
    ) -> Self {
        match search.phase() {
            DocSearchPhase::Loading => Self::Loading,
            DocSearchPhase::Searching(peers) => Self::Requesting(Request::new(
                peers.clone(),
                search.pending_connections().to_vec(),
            )),
            DocSearchPhase::Ready => {
                if let Some(actor) = actors.get(&actor_id) {
                    Self::Found(actor.doc.clone())
                } else {
                    tracing::warn!(
                        ?actor_id,
                        "search reports Ready but no DocHandle is registered"
                    );
                    SearchState::Requesting(Request::empty())
                }
            }
        }
    }
}

/// An ongoing request for a document
///
/// When a document is in a "requesting" state the Repo will automatically send
/// a request for the document to any peer who is allowed by the
/// [`AnnouncePolicy`]. If a peer responds with the document the repo will wait
/// until we have synced up to the initial heads the peer in question responded
/// with before transitioning to a [`DocSearchPhase::Ready`] state.
///
/// The state of each peer is tracked in the [`peers`] map, keyed by
/// [`ConnectionId`]. If you want to show information about the peer in question
/// the connection ID can be used to look up the connection state in the
/// [`Repo::connected_peers`].
///
/// The request also has a list of `pending_dialers` that are dialers we are
/// currently waiting for connection to complete on (either they are in the
/// process of establishing a connection, or performing a handshake). This is
/// useful because sometimes you want to wait until all the pending dialers have
/// completed before displaying a message to the user that a document cannot be
/// found. This common case is captured in the [`is_currently_unavailable`]
/// method.
#[derive(Debug, Clone)]
pub struct Request {
    peers: HashMap<ConnectionId, PeerRequestState>,
    pending_dialers: Vec<url::Url>,
}

impl Request {
    pub(crate) fn empty() -> Self {
        Self {
            peers: HashMap::new(),
            pending_dialers: Vec::new(),
        }
    }

    pub(crate) fn new(
        peers: HashMap<ConnectionId, PeerRequestState>,
        pending_dialers: Vec<url::Url>,
    ) -> Self {
        Self {
            peers,
            pending_dialers,
        }
    }

    /// A convenience method that returns `true` if all peers have said they
    /// don't have the document and there are no pending dialers
    pub fn is_currently_unavailable(&self) -> bool {
        self.peers.values().all(|state| {
            matches!(state, PeerRequestState::Unavailable) && !self.pending_dialers.is_empty()
        })
    }

    /// The peers we are currently requesting the document from
    pub fn peers(&self) -> &HashMap<ConnectionId, PeerRequestState> {
        &self.peers
    }

    /// Any dialers that are pending
    pub fn pending_dialers(&self) -> &[url::Url] {
        &self.pending_dialers
    }
}
