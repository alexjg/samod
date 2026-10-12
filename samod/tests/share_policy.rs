use std::{cell::Cell, rc::Rc, sync::Arc};

use automerge::Automerge;
use futures::{Stream, StreamExt, channel::mpsc, executor::LocalPool, stream};
use samod::{
    BackoffConfig, DocumentId, LocalSharePolicy, PeerId, Repo, ShareConfig,
    transport::channel::ChannelDialer,
};

#[derive(Clone)]
struct LocalPolicy(
    Rc<Cell<usize>>,
    mpsc::UnboundedSender<mpsc::UnboundedSender<ShareConfig>>,
);

impl LocalSharePolicy for LocalPolicy {
    fn share(
        &self,
        _doc_id: DocumentId,
        _peer_id: PeerId,
    ) -> impl Stream<Item = ShareConfig> + 'static {
        let calls = self.0.clone();
        let (tx, rx) = mpsc::unbounded();
        self.1.unbounded_send(tx).unwrap();
        stream::once(async move {
            calls.set(calls.get() + 1);
            ShareConfig::read_only()
        })
        .chain(rx)
    }
}

#[test]
fn non_send_share_policy_works_on_localpool() {
    let mut pool = LocalPool::new();
    let spawner = pool.spawner();
    pool.run_until(async move {
        let calls = Rc::new(Cell::new(0));
        let (tx, mut subscriptions) = mpsc::unbounded();
        let publisher = Repo::build_localpool(spawner.clone())
            .with_share_policy(LocalPolicy(calls.clone(), tx))
            .load_local()
            .await;
        let reader = Repo::build_localpool(spawner)
            .with_peer_id("reader".into())
            .load_local()
            .await;
        let doc = publisher.create(Automerge::new()).await.unwrap();
        let acceptor = reader
            .make_acceptor("channel://reader".parse().unwrap())
            .unwrap();
        let dialer = publisher
            .dial(
                BackoffConfig::default(),
                Arc::new(ChannelDialer::new(acceptor)),
            )
            .unwrap();
        dialer.established().await.unwrap();
        let reader_doc = reader
            .find(doc.document_id().clone())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(calls.get(), 1);
        assert!(
            reader_doc
                .peers()
                .0
                .values()
                .any(|state| state.peer_read_only)
        );
        let decisions = subscriptions.next().await.unwrap();
        decisions.unbounded_send(ShareConfig::read_write()).unwrap();
        let (mut peers, mut updates) = doc.peers();
        while peers
            .values()
            .any(|peer| peer.share_config != Some(ShareConfig::read_write()))
        {
            peers.extend(updates.next().await.unwrap());
        }
        assert_eq!(calls.get(), 1, "updates must not resubscribe");
        publisher.stop().await;
        reader.stop().await;
    });
}

#[cfg(feature = "tokio")]
mod tokio_tests {
    use super::*;
    use futures::{
        FutureExt, StreamExt,
        channel::{mpsc, oneshot},
        future::{BoxFuture, Shared},
    };
    use samod::SharePolicy;
    use std::time::Duration;

    #[derive(Clone)]
    struct DelayedPolicy {
        started: mpsc::UnboundedSender<()>,
        decision: Shared<BoxFuture<'static, ShareConfig>>,
    }

    impl SharePolicy for DelayedPolicy {
        fn share(
            &self,
            _doc: DocumentId,
            _peer: PeerId,
        ) -> impl Stream<Item = ShareConfig> + Send + 'static {
            self.started.unbounded_send(()).unwrap();
            stream::once(self.decision.clone())
        }
    }

    #[tokio::test]
    async fn asynchronous_share_policy_is_awaited() {
        tokio::time::timeout(Duration::from_secs(5), async {
            let (decision_tx, decision_rx) = oneshot::channel();
            let (started_tx, mut started_rx) = mpsc::unbounded();
            let publisher = Repo::build_tokio()
                .with_share_policy(DelayedPolicy {
                    started: started_tx,
                    decision: decision_rx.map(Result::unwrap).boxed().shared(),
                })
                .load()
                .await;
            let reader = Repo::build_tokio().load().await;
            let doc = publisher.create(Automerge::new()).await.unwrap();
            let acceptor = reader
                .make_acceptor("channel://reader".parse().unwrap())
                .unwrap();
            let dialer = publisher
                .dial(
                    BackoffConfig::default(),
                    Arc::new(ChannelDialer::new(acceptor)),
                )
                .unwrap();
            dialer.established().await.unwrap();
            started_rx.next().await.unwrap();
            assert!(
                doc.peers()
                    .0
                    .values()
                    .all(|state| state.share_config.is_none())
            );
            decision_tx.send(ShareConfig::read_only()).unwrap();
            let reader_doc = reader
                .find(doc.document_id().clone())
                .await
                .unwrap()
                .unwrap();
            assert!(
                reader_doc
                    .peers()
                    .0
                    .values()
                    .any(|state| state.peer_read_only)
            );
            publisher.stop().await;
            reader.stop().await;
        })
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn find_finishes_when_all_sources_are_excluded_by_policy() {
        tokio::time::timeout(Duration::from_secs(5), async {
            let alice = Repo::build_tokio()
                .with_share_policy(ShareConfig::read_only())
                .load()
                .await;
            let bob = Repo::build_tokio().load().await;
            let doc = bob.create(Automerge::new()).await.unwrap();
            let acceptor = bob.make_acceptor("channel://bob".parse().unwrap()).unwrap();
            let dialer = alice
                .dial(
                    BackoffConfig::default(),
                    Arc::new(ChannelDialer::new(acceptor)),
                )
                .unwrap();
            dialer.established().await.unwrap();
            assert!(
                alice
                    .find(doc.document_id().clone())
                    .await
                    .unwrap()
                    .is_none()
            );
            alice.stop().await;
            bob.stop().await;
        })
        .await
        .unwrap();
    }
}
