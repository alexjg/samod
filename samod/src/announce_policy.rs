use samod_core::{DocumentId, PeerId};

/// Whether to announce a document to a peer
///
/// To configure announcement behavior implement this trait and pass the
/// implementation to
/// [`RepoBuilder::with_announce_policy`](crate::RepoBuilder::with_announce_policy).
/// Note that the trait is implemented for `Fn(DocumentId, PeerId) -> bool`, so
/// a closure can be passed directly in many cases.
pub trait AnnouncePolicy: Clone + Send + 'static {
    /// Whether we should announce the given document to the given peer ID. This
    /// is used like so:
    ///
    /// * When we connect to a new peer we check this for each
    ///   [`DocHandle`](crate::DocHandle) we are synchronizing
    /// * When we create a new [`DocHandle`](crate::DocHandle) we check this for
    ///   each connected peer
    /// * When we request a new document using [`Repo::find`](crate::Repo::find)
    ///   we check this for each connected peer before sending a request (this
    ///   prevents leaking document IDs)
    ///
    /// Note that the peer IDs are not authenticated by the network protocol
    /// `samod` implements, so if you are relying on this method for
    /// authorization you must make sure that the network layer you provide is
    /// doing authentication in it's own fashion somehow.
    fn should_announce(
        &self,
        doc_id: DocumentId,
        peer_id: PeerId,
    ) -> impl Future<Output = bool> + Send + 'static;
}

impl<F> AnnouncePolicy for F
where
    F: Fn(DocumentId, PeerId) -> bool + Clone + Send + 'static,
{
    fn should_announce(
        &self,
        doc_id: DocumentId,
        peer_id: PeerId,
    ) -> impl Future<Output = bool> + Send + 'static {
        let result = self(doc_id, peer_id);
        async move { result }
    }
}

/// Always announce every documents to every peer
#[derive(Clone)]
pub struct AlwaysAnnounce;

impl AnnouncePolicy for AlwaysAnnounce {
    #[allow(clippy::manual_async_fn)]
    fn should_announce(
        &self,
        _doc_id: DocumentId,
        _peer_id: PeerId,
    ) -> impl Future<Output = bool> + Send + 'static {
        async { true }
    }
}
