use futures::{Stream, stream};
use samod_core::{DocumentId, PeerId, ShareConfig};

/// Decide how a document is shared with a remote peer, and publish updates.
///
/// Configure this with [`RepoBuilder::with_share_policy`](crate::RepoBuilder::with_share_policy).
/// One stream is subscribed to per active document/peer pair, shared by all
/// connections to that peer. Sharing waits for the first item. Subsequent items
/// update permissions automatically; the last decision remains active while
/// waiting for another item. Identical consecutive decisions are ignored. Updates
/// are latest-decision-wins: a newer item may supersede an earlier one before it
/// is installed on the document's connections.
///
/// When a stream ends, its last decision remains in force. A stream which ends
/// without yielding a decision denies access. The subscription is dropped when
/// the document no longer has connections to that peer, or the repo stops. A
/// later connection starts a new subscription, so streams must provide their
/// current decision as well as future updates.
///
/// Yield the initial decision promptly: incoming sync messages are buffered in
/// bounded per-document/connection queues, and overflowing a queue closes the
/// connection. Updates cannot retract accepted changes or messages already
/// queued for transport. A completed `find()` is not revived, but an ongoing
/// [`Repo::search`](crate::Repo::search) can resume after a grant.
///
/// The default is [`ShareConfig::read_write`]. A [`ShareConfig`] itself can be
/// used as a constant policy. Closures returning streams are also supported;
/// use [`futures::stream::once`] for a single asynchronous decision.
///
/// Access describes the **remote peer's** permissions: read-only sends our
/// changes but ignores theirs. It does not prevent local edits. Write-only
/// sharing is not supported. Ephemera are allowed in both directions for
/// read-only and read-write peers, but not for denied peers.
///
/// Peer IDs must be authenticated by the transport if this is used for
/// authorization; see [`Transport::with_expected_peer_id`](crate::Transport::with_expected_peer_id).
/// Permissions apply to the delivering connection, not the original change
/// author, so an allowed writer may relay changes from another peer.
///
/// # Example
///
/// ```rust,no_run
/// # #[cfg(feature = "tokio")]
/// # async fn example() {
/// use samod::{PeerId, Repo, ShareConfig};
/// let writer = PeerId::from("writer");
/// let repo = Repo::build_tokio()
///     .with_share_policy(move |_doc_id, peer_id| {
///         let config = if peer_id == writer {
///             ShareConfig::read_write()
///         } else {
///             ShareConfig::read_only().with_announce(false)
///         };
///         futures::stream::iter([config])
///     })
///     .load().await;
/// # }
/// ```
pub trait SharePolicy: Clone + Send + 'static {
    fn share(
        &self,
        doc_id: DocumentId,
        peer_id: PeerId,
    ) -> impl Stream<Item = ShareConfig> + Send + 'static;
}

/// A version of [`SharePolicy`] for runtimes which do not require `Send` streams.
pub trait LocalSharePolicy: Clone + 'static {
    fn share(
        &self,
        doc_id: DocumentId,
        peer_id: PeerId,
    ) -> impl Stream<Item = ShareConfig> + 'static;
}

impl<A: SharePolicy> LocalSharePolicy for A {
    fn share(
        &self,
        doc_id: DocumentId,
        peer_id: PeerId,
    ) -> impl Stream<Item = ShareConfig> + 'static {
        SharePolicy::share(self, doc_id, peer_id)
    }
}

impl<F, S> SharePolicy for F
where
    F: Fn(DocumentId, PeerId) -> S + Clone + Send + 'static,
    S: Stream<Item = ShareConfig> + Send + 'static,
{
    fn share(
        &self,
        doc_id: DocumentId,
        peer_id: PeerId,
    ) -> impl Stream<Item = ShareConfig> + Send + 'static {
        self(doc_id, peer_id)
    }
}

impl SharePolicy for ShareConfig {
    fn share(
        &self,
        _doc_id: DocumentId,
        _peer_id: PeerId,
    ) -> impl Stream<Item = ShareConfig> + Send + 'static {
        stream::iter([*self])
    }
}
