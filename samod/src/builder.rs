use std::sync::Arc;

use samod_core::{PeerId, ShareConfig};

use crate::{
    Repo,
    observer::RepoObserver,
    runtime::{LocalRuntimeHandle, RuntimeHandle},
    share_policy::{LocalSharePolicy, SharePolicy},
    storage::{InMemoryStorage, LocalStorage, Storage},
};

/// How to run concurrent documents
///
/// See the [concurrency section](./index.html#concurrency) of the module level
/// documentation
pub enum ConcurrencyConfig {
    /// Run each document in a separate task on the async runtime
    AsyncRuntime,
    /// Run each document on a rayon threadpool
    #[cfg(feature = "threadpool")]
    Threadpool(rayon::ThreadPool),
}

/// A struct for configuring a [`Repo`](crate::Repo)
///
/// ## `Send` and non-`Send` futures
///
/// Once you've finished configuring the builder, you call either [`load`](Self::load) or
/// [`load_local`](Self::load_local) to create a [`Repo`]. The difference between the two is that
/// [`load`](Self::load) is only available for runtimes which implement `RuntimeHandle` and
/// thus require `Send` futures (such as tokio). [`load_local`](Self::load_local) on the other
/// hand works with runtimes that implement [`LocalRuntimeHandle`] and thus does
/// not require [`Send`] futures. If you want to use
/// [`load_local`](Self::load_local) you need to configure the storage and
/// share policy implementations to be implementations of [`LocalStorage`]
/// and [`LocalSharePolicy`] respectively. See the [module level
/// documentation on runtimes](./index.html#runtimes) for more details.
pub struct RepoBuilder<S, R, A> {
    pub(crate) storage: S,
    pub(crate) runtime: R,
    pub(crate) share_policy: A,
    pub(crate) peer_id: Option<PeerId>,
    pub(crate) concurrency: ConcurrencyConfig,
    pub(crate) observer: Option<Arc<dyn RepoObserver>>,
}

impl<S, R, A> RepoBuilder<S, R, A> {
    pub fn with_storage<S2>(self, storage: S2) -> RepoBuilder<S2, R, A> {
        RepoBuilder {
            storage,
            peer_id: self.peer_id,
            runtime: self.runtime,
            share_policy: self.share_policy,
            concurrency: self.concurrency,
            observer: self.observer,
        }
    }

    pub fn with_runtime<R2>(self, runtime: R2) -> RepoBuilder<S, R2, A> {
        RepoBuilder {
            runtime,
            peer_id: self.peer_id,
            storage: self.storage,
            share_policy: self.share_policy,
            concurrency: self.concurrency,
            observer: self.observer,
        }
    }

    pub fn with_peer_id(mut self, peer_id: PeerId) -> Self {
        self.peer_id = Some(peer_id);
        self
    }

    /// Configure document access and announcements for each remote peer.
    /// Defaults to read-write access with announcements enabled.
    pub fn with_share_policy<A2>(self, share_policy: A2) -> RepoBuilder<S, R, A2> {
        RepoBuilder {
            runtime: self.runtime,
            peer_id: self.peer_id,
            storage: self.storage,
            share_policy,
            concurrency: self.concurrency,
            observer: self.observer,
        }
    }

    /// Configure how the repository should process concurrent documents
    ///
    /// See the [concurrency section](./index.html#concurrency) of the module level
    /// documentation
    pub fn with_concurrency(mut self, concurrency: ConcurrencyConfig) -> Self {
        self.concurrency = concurrency;
        self
    }

    /// Set an observer for receiving structured observability events.
    pub fn with_observer(mut self, observer: impl RepoObserver) -> Self {
        self.observer = Some(Arc::new(observer));
        self
    }
}

impl<R> RepoBuilder<InMemoryStorage, R, ShareConfig> {
    pub fn new(runtime: R) -> RepoBuilder<InMemoryStorage, R, ShareConfig> {
        RepoBuilder {
            storage: InMemoryStorage::new(),
            runtime,
            peer_id: None,
            share_policy: ShareConfig::default(),
            concurrency: ConcurrencyConfig::AsyncRuntime,
            observer: None,
        }
    }
}

impl<S: Storage, R: RuntimeHandle + Clone + Send, A: SharePolicy> RepoBuilder<S, R, A> {
    /// Create the repository
    pub async fn load(self) -> Repo {
        Repo::load(self).await
    }
}

impl<S: LocalStorage, R: LocalRuntimeHandle + Clone + 'static, A: LocalSharePolicy>
    RepoBuilder<S, R, A>
{
    /// Create the repository
    pub async fn load_local(self) -> Repo {
        Repo::load_local(self).await
    }
}
