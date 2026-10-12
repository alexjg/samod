#![cfg(feature = "tokio")]

use automerge::{Automerge, ROOT, ReadDoc, transaction::Transactable};
use futures::{
    FutureExt, Stream, StreamExt,
    channel::{mpsc, oneshot},
};
use samod::{
    BackoffConfig, DialerHandle, DocHandle, DocumentId, PeerId, Repo, SearchState, ShareConfig,
    SharePolicy, transport::channel::ChannelDialer,
};
use std::{
    pin::Pin,
    sync::Arc,
    task::{Context, Poll},
    time::Duration,
};

#[derive(Clone)]
struct ControlledPolicy(mpsc::UnboundedSender<Subscription>);

struct Subscription {
    document_id: DocumentId,
    peer_id: PeerId,
    updates: mpsc::UnboundedSender<ShareConfig>,
    closed: oneshot::Receiver<()>,
}

struct Decisions {
    updates: mpsc::UnboundedReceiver<ShareConfig>,
    closed: Option<oneshot::Sender<()>>,
}

impl Stream for Decisions {
    type Item = ShareConfig;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        Pin::new(&mut self.updates).poll_next(cx)
    }
}

impl Drop for Decisions {
    fn drop(&mut self) {
        let _ = self.closed.take().unwrap().send(());
    }
}

impl SharePolicy for ControlledPolicy {
    fn share(
        &self,
        document_id: DocumentId,
        peer_id: PeerId,
    ) -> impl Stream<Item = ShareConfig> + Send + 'static {
        let (tx, updates) = mpsc::unbounded();
        let (closed, rx) = oneshot::channel();
        self.0
            .unbounded_send(Subscription {
                document_id,
                peer_id,
                updates: tx,
                closed: rx,
            })
            .unwrap();
        Decisions {
            updates,
            closed: Some(closed),
        }
    }
}

async fn connect(publisher: &Repo, reader: &Repo) -> DialerHandle {
    let acceptor = reader
        .make_acceptor(
            format!("channel://reader/{}", rand::random::<u64>())
                .parse()
                .unwrap(),
        )
        .unwrap();
    let dialer = publisher
        .dial(
            BackoffConfig::default(),
            Arc::new(ChannelDialer::new(acceptor)),
        )
        .unwrap();
    dialer.established().await.unwrap();
    dialer
}

async fn wait_for_config(doc: &DocHandle, config: ShareConfig) {
    let (mut peers, mut updates) = doc.peers();
    while peers.is_empty() || peers.values().any(|peer| peer.share_config != Some(config)) {
        updates.next().await.unwrap();
        peers = doc.peers().0;
    }
}

async fn wait_for_peer_count(doc: &DocHandle, count: usize) {
    // Removal is not represented in the peer-state delta stream.
    while doc.peers().0.len() != count {
        tokio::task::yield_now().await;
    }
}

async fn wait_for_key(doc: &DocHandle, key: &str) {
    let mut changes = Box::pin(doc.changes());
    while !doc.with_document(|doc| doc.get(ROOT, key).unwrap().is_some()) {
        changes.next().await.unwrap();
    }
}

async fn pair() -> (Repo, Repo, DocHandle, mpsc::UnboundedReceiver<Subscription>) {
    pair_with_threadpool(false).await
}

async fn pair_with_threadpool(
    threadpool: bool,
) -> (Repo, Repo, DocHandle, mpsc::UnboundedReceiver<Subscription>) {
    let (tx, subscriptions) = mpsc::unbounded();
    let builder = Repo::build_tokio().with_share_policy(ControlledPolicy(tx));
    #[cfg(feature = "threadpool")]
    let builder = if threadpool {
        builder.with_concurrency(samod::ConcurrencyConfig::Threadpool(
            rayon::ThreadPoolBuilder::new()
                .num_threads(2)
                .build()
                .unwrap(),
        ))
    } else {
        builder
    };
    let _ = threadpool;
    let publisher = builder.load().await;
    let reader = Repo::build_tokio()
        .with_peer_id("reader".into())
        .load()
        .await;
    let doc = publisher.create(Automerge::new()).await.unwrap();
    (publisher, reader, doc, subscriptions)
}

async fn live_grants(threadpool: bool) {
    let (tx, mut subscriptions) = mpsc::unbounded();
    let builder = Repo::build_tokio()
        .with_peer_id("publisher".into())
        .with_share_policy(ControlledPolicy(tx));
    #[cfg(feature = "threadpool")]
    let builder = if threadpool {
        builder.with_concurrency(samod::ConcurrencyConfig::Threadpool(
            rayon::ThreadPoolBuilder::new()
                .num_threads(2)
                .build()
                .unwrap(),
        ))
    } else {
        builder
    };
    let _ = threadpool;
    let publisher = builder.load().await;
    let reader = Repo::build_tokio()
        .with_peer_id("reader".into())
        .load()
        .await;
    let doc = publisher.create(Automerge::new()).await.unwrap();
    connect(&publisher, &reader).await;
    let subscription = subscriptions.next().await.unwrap();
    assert_eq!(&subscription.document_id, doc.document_id());
    assert_eq!(subscription.peer_id, PeerId::from("reader"));
    assert!(
        doc.peers()
            .0
            .values()
            .all(|peer| peer.share_config.is_none())
    );
    subscription
        .updates
        .unbounded_send(ShareConfig::none())
        .unwrap();
    wait_for_config(&doc, ShareConfig::none()).await;
    assert!(
        reader
            .find(doc.document_id().clone())
            .await
            .unwrap()
            .is_none()
    );
    let (_, mut search) = reader.search(doc.document_id().clone()).unwrap();

    let read_only = ShareConfig::read_only().with_announce(false);
    subscription.updates.unbounded_send(read_only).unwrap();
    wait_for_config(&doc, read_only).await;
    let reader_doc = loop {
        if let SearchState::Found(doc) = search.next().await.unwrap() {
            break doc;
        }
    };
    // The source is now pending, but sharing must continue using its last item.
    doc.with_document(|doc| {
        doc.transact::<_, _, automerge::AutomergeError>(|tx| tx.put(ROOT, "publisher_edit", true))
            .unwrap();
    });
    wait_for_key(&reader_doc, "publisher_edit").await;
    assert!(
        doc.peers()
            .0
            .values()
            .all(|peer| peer.share_config == Some(read_only))
    );
    reader_doc.with_document(|doc| {
        doc.transact::<_, _, automerge::AutomergeError>(|tx| tx.put(ROOT, "reader_edit", true))
            .unwrap();
    });
    assert!(!doc.with_document(|doc| doc.get(ROOT, "reader_edit").unwrap().is_some()));
    subscription
        .updates
        .unbounded_send(ShareConfig::read_write().with_announce(false))
        .unwrap();
    wait_for_key(&doc, "reader_edit").await;
    assert!(
        subscriptions.next().now_or_never().is_none(),
        "updates must not resubscribe"
    );
    publisher.stop().await;
    subscription.closed.await.unwrap();
    reader.stop().await;
}

#[tokio::test]
async fn stream_updates_resume_live_sync() {
    tokio::time::timeout(Duration::from_secs(5), live_grants(false))
        .await
        .unwrap();
}

#[tokio::test]
#[cfg(feature = "threadpool")]
async fn stream_updates_work_with_threadpool_actors() {
    tokio::time::timeout(Duration::from_secs(5), live_grants(true))
        .await
        .unwrap();
}

#[tokio::test]
async fn stopping_drops_pending_initial_and_live_streams() {
    tokio::time::timeout(Duration::from_secs(5), async {
        for initial in [None, Some(ShareConfig::read_write())] {
            let (publisher, reader, doc, mut subscriptions) = pair().await;
            connect(&publisher, &reader).await;
            let subscription = subscriptions.next().await.unwrap();
            if let Some(initial) = initial {
                subscription.updates.unbounded_send(initial).unwrap();
                wait_for_config(&doc, initial).await;
                // Also leave a changed decision queued as shutdown begins.
                subscription
                    .updates
                    .unbounded_send(ShareConfig::none())
                    .unwrap();
            }
            publisher.stop().await;
            subscription.closed.await.unwrap();
            assert!(
                subscription
                    .updates
                    .unbounded_send(ShareConfig::read_write())
                    .is_err()
            );
            reader.stop().await;
        }
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn stream_end_retains_last_decision_or_denies_an_empty_stream() {
    tokio::time::timeout(Duration::from_secs(5), async {
        for initial in [
            None,
            Some(ShareConfig::read_only()),
            Some(ShareConfig::read_write()),
        ] {
            let (publisher, reader, doc, mut subscriptions) = pair().await;
            connect(&publisher, &reader).await;
            let subscription = subscriptions.next().await.unwrap();
            if let Some(initial) = initial {
                subscription.updates.unbounded_send(initial).unwrap();
            }
            drop(subscription.updates);
            subscription.closed.await.unwrap();
            let config = initial.unwrap_or(ShareConfig::none());
            wait_for_config(&doc, config).await;
            let found = reader.find(doc.document_id().clone()).await.unwrap();
            assert_eq!(found.is_some(), initial.is_some());

            // A second connection uses the cached final value, not a new stream.
            connect(&publisher, &reader).await;
            wait_for_peer_count(&doc, 2).await;
            wait_for_config(&doc, config).await;
            assert!(subscriptions.next().now_or_never().is_none());
            publisher.stop().await;
            reader.stop().await;
        }
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn one_subscription_covers_all_connections_and_reconnection_starts_fresh() {
    tokio::time::timeout(Duration::from_secs(5), async {
        let (publisher, reader, doc, mut subscriptions) = pair().await;
        let first = connect(&publisher, &reader).await;
        let mut subscription = subscriptions.next().await.unwrap();
        let second = connect(&publisher, &reader).await;
        wait_for_peer_count(&doc, 2).await;
        subscription
            .updates
            .unbounded_send(ShareConfig::read_only())
            .unwrap();
        wait_for_config(&doc, ShareConfig::read_only()).await;
        assert!(subscriptions.next().now_or_never().is_none());
        subscription
            .updates
            .unbounded_send(ShareConfig::read_write())
            .unwrap();
        wait_for_config(&doc, ShareConfig::read_write()).await;

        first.close();
        wait_for_peer_count(&doc, 1).await;
        assert!((&mut subscription.closed).now_or_never().is_none());
        subscription
            .updates
            .unbounded_send(ShareConfig::none())
            .unwrap();
        wait_for_config(&doc, ShareConfig::none()).await;
        second.close();
        subscription.closed.await.unwrap();
        assert!(
            subscription
                .updates
                .unbounded_send(ShareConfig::read_write())
                .is_err()
        );

        connect(&publisher, &reader).await;
        let next = subscriptions.next().await.unwrap();
        assert_eq!(&next.document_id, doc.document_id());
        assert!(
            doc.peers()
                .0
                .values()
                .all(|peer| peer.share_config.is_none())
        );
        next.updates
            .unbounded_send(ShareConfig::read_only())
            .unwrap();
        wait_for_config(&doc, ShareConfig::read_only()).await;
        publisher.stop().await;
        next.closed.await.unwrap();
        reader.stop().await;
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn disconnect_cancels_a_stream_before_its_first_item() {
    tokio::time::timeout(Duration::from_secs(5), async {
        let (publisher, reader, doc, mut subscriptions) = pair().await;
        let first = connect(&publisher, &reader).await;
        let old = subscriptions.next().await.unwrap();
        first.close();
        old.closed.await.unwrap();
        connect(&publisher, &reader).await;
        let next = subscriptions.next().await.unwrap();
        assert!(
            old.updates
                .unbounded_send(ShareConfig::read_write())
                .is_err()
        );
        next.updates.unbounded_send(ShareConfig::none()).unwrap();
        wait_for_config(&doc, ShareConfig::none()).await;
        publisher.stop().await;
        next.closed.await.unwrap();
        reader.stop().await;
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn a_pending_stream_does_not_block_other_documents() {
    tokio::time::timeout(Duration::from_secs(5), async {
        let (publisher, reader, first_doc, mut subscriptions) = pair().await;
        let second_doc = publisher.create(Automerge::new()).await.unwrap();
        assert!(
            subscriptions.next().now_or_never().is_none(),
            "inactive pairs have no stream"
        );
        connect(&publisher, &reader).await;
        let first = subscriptions.next().await.unwrap();
        let second = subscriptions.next().await.unwrap();
        let (pending, active) = if first.document_id == *first_doc.document_id() {
            (first, second)
        } else {
            (second, first)
        };
        assert_eq!(&active.document_id, second_doc.document_id());
        active
            .updates
            .unbounded_send(ShareConfig::read_write())
            .unwrap();
        assert!(
            reader
                .find(second_doc.document_id().clone())
                .await
                .unwrap()
                .is_some()
        );
        assert!(
            first_doc
                .peers()
                .0
                .values()
                .all(|peer| peer.share_config.is_none())
        );
        publisher.stop().await;
        pending.closed.await.unwrap();
        active.closed.await.unwrap();
        reader.stop().await;
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn rapid_updates_install_the_final_decision() {
    rapid_updates(false).await;
}

#[cfg(feature = "threadpool")]
#[tokio::test]
async fn rapid_updates_install_the_final_decision_with_threadpool_actors() {
    rapid_updates(true).await;
}

async fn rapid_updates(threadpool: bool) {
    tokio::time::timeout(Duration::from_secs(5), async {
        let (publisher, reader, doc, mut subscriptions) = pair_with_threadpool(threadpool).await;
        connect(&publisher, &reader).await;
        let subscription = subscriptions.next().await.unwrap();
        for _ in 0..20 {
            for config in [
                ShareConfig::none(),
                ShareConfig::read_only(),
                ShareConfig::read_write(),
            ] {
                subscription.updates.unbounded_send(config).unwrap();
            }
        }
        drop(subscription.updates);
        subscription.closed.await.unwrap();
        wait_for_config(&doc, ShareConfig::read_write()).await;
        assert!(
            reader
                .find(doc.document_id().clone())
                .await
                .unwrap()
                .is_some()
        );
        assert!(subscriptions.next().now_or_never().is_none());
        publisher.stop().await;
        reader.stop().await;
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn always_ready_duplicate_decisions_do_not_starve_the_executor() {
    // Use a current-thread runtime: the reader and document actors cannot make
    // progress unless the publisher's IO loop actually yields to the executor.
    tokio::time::timeout(Duration::from_secs(5), async {
        let publisher = Repo::build_tokio()
            .with_share_policy(|_, _| {
                let mut items = 0;
                futures::stream::repeat(ShareConfig::read_only()).inspect(move |_| {
                    items += 1;
                    // Bound a busy-loop regression rather than hanging the
                    // test runner (which would also prevent its timeout firing).
                    assert!(items < 10_000, "policy stream starved other tasks");
                })
            })
            .load()
            .await;
        let reader = Repo::build_tokio().load().await;
        let doc = publisher.create(Automerge::new()).await.unwrap();
        connect(&publisher, &reader).await;
        assert!(
            reader
                .find(doc.document_id().clone())
                .await
                .unwrap()
                .is_some()
        );
        publisher.stop().await;
        reader.stop().await;
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn identical_items_do_not_pause_sharing() {
    tokio::time::timeout(Duration::from_secs(5), async {
        let (publisher, reader, doc, mut subscriptions) = pair().await;
        connect(&publisher, &reader).await;
        let subscription = subscriptions.next().await.unwrap();
        subscription
            .updates
            .unbounded_send(ShareConfig::read_only())
            .unwrap();
        wait_for_config(&doc, ShareConfig::read_only()).await;
        let (_, mut peer_updates) = doc.peers();
        for _ in 0..10 {
            subscription
                .updates
                .unbounded_send(ShareConfig::read_only())
                .unwrap();
        }
        drop(subscription.updates);
        subscription.closed.await.unwrap();
        while let Some(Some(update)) = peer_updates.next().now_or_never() {
            assert!(
                update
                    .values()
                    .all(|peer| peer.share_config == Some(ShareConfig::read_only()))
            );
        }
        publisher.stop().await;
        reader.stop().await;
    })
    .await
    .unwrap();
}
