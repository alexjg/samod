use samod_core::PeerId;

use crate::{
    Samod,
    announce_policy::{AlwaysAnnounce, AnnouncePolicy},
    runtime::RuntimeHandle,
    storage::{InMemoryStorage, Storage},
};

pub struct SamodBuilder<S, R, A> {
    pub(crate) storage: S,
    pub(crate) runtime: R,
    pub(crate) announce_policy: A,
    pub(crate) peer_id: Option<PeerId>,
}

impl<S, R, A> SamodBuilder<S, R, A> {
    pub fn with_storage<S2: Storage>(self, storage: S2) -> SamodBuilder<S2, R, A> {
        SamodBuilder {
            storage,
            peer_id: self.peer_id,
            runtime: self.runtime,
            announce_policy: self.announce_policy,
        }
    }

    pub fn with_runtime<R2: RuntimeHandle>(self, runtime: R2) -> SamodBuilder<S, R2, A> {
        SamodBuilder {
            runtime,
            peer_id: self.peer_id,
            storage: self.storage,
            announce_policy: self.announce_policy,
        }
    }

    pub fn with_peer_id(mut self, peer_id: PeerId) -> Self {
        self.peer_id = Some(peer_id);
        self
    }

    pub fn with_announce_policy<A2: AnnouncePolicy>(
        self,
        announce_policy: A2,
    ) -> SamodBuilder<S, R, A2> {
        SamodBuilder {
            runtime: self.runtime,
            peer_id: self.peer_id,
            storage: self.storage,
            announce_policy,
        }
    }
}

impl<R> SamodBuilder<InMemoryStorage, R, AlwaysAnnounce> {
    pub fn new(runtime: R) -> SamodBuilder<InMemoryStorage, R, AlwaysAnnounce> {
        SamodBuilder {
            storage: InMemoryStorage::new(),
            runtime,
            peer_id: None,
            announce_policy: AlwaysAnnounce,
        }
    }
}

impl<S: Storage, R: RuntimeHandle, A: AnnouncePolicy> SamodBuilder<S, R, A> {
    pub async fn load(self) -> Samod {
        Samod::load(self).await
    }
}
