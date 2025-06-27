use samod_core::{DocumentId, PeerId};

pub trait AnnouncePolicy: Clone + Send + 'static {
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
