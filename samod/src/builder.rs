use samod_core::PeerId;

use crate::{
    Repo,
    announce_policy::{AlwaysAnnounce, AnnouncePolicy},
    runtime::RuntimeHandle,
    storage::{InMemoryStorage, Storage},
};

/// A struct for configuring a [`Repo`](crate::Repo)
pub struct RepoBuilder<S, R, A> {
    pub(crate) storage: S,
    pub(crate) runtime: R,
    pub(crate) announce_policy: A,
    pub(crate) peer_id: Option<PeerId>,
    pub(crate) threadpool: Option<rayon::ThreadPool>,
}

impl<S, R, A> RepoBuilder<S, R, A> {
    pub fn with_storage<S2: Storage>(self, storage: S2) -> RepoBuilder<S2, R, A> {
        RepoBuilder {
            storage,
            peer_id: self.peer_id,
            runtime: self.runtime,
            announce_policy: self.announce_policy,
            threadpool: self.threadpool,
        }
    }

    pub fn with_runtime<R2: RuntimeHandle>(self, runtime: R2) -> RepoBuilder<S, R2, A> {
        RepoBuilder {
            runtime,
            peer_id: self.peer_id,
            storage: self.storage,
            announce_policy: self.announce_policy,
            threadpool: self.threadpool,
        }
    }

    pub fn with_peer_id(mut self, peer_id: PeerId) -> Self {
        self.peer_id = Some(peer_id);
        self
    }

    pub fn with_announce_policy<A2: AnnouncePolicy>(
        self,
        announce_policy: A2,
    ) -> RepoBuilder<S, R, A2> {
        RepoBuilder {
            runtime: self.runtime,
            peer_id: self.peer_id,
            storage: self.storage,
            announce_policy,
            threadpool: self.threadpool,
        }
    }

    pub fn with_threadpool(mut self, threadpool: Option<rayon::ThreadPool>) -> Self {
        self.threadpool = threadpool;
        self
    }
}

impl<R> RepoBuilder<InMemoryStorage, R, AlwaysAnnounce> {
    pub fn new(runtime: R) -> RepoBuilder<InMemoryStorage, R, AlwaysAnnounce> {
        RepoBuilder {
            storage: InMemoryStorage::new(),
            runtime,
            peer_id: None,
            announce_policy: AlwaysAnnounce,
            threadpool: None,
        }
    }
}

impl<S: Storage, R: RuntimeHandle, A: AnnouncePolicy> RepoBuilder<S, R, A> {
    /// Create the repository
    pub async fn load(self) -> Repo {
        Repo::load(self).await
    }
}
