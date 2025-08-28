use std::sync::{Arc, Mutex};

use samod_core::{DocumentActorId, DocumentId, actors::document::DocActorResult};

use crate::{actor_task::ActorTask, doc_actor_inner::DocActorInner};

/// Enum representing the two possible ways of running document actors
pub(crate) enum DocRunner {
    /// Run the actors on a threadpool
    Threadpool(rayon::ThreadPool),
    /// Run the actors on an async task which is listening on the other end of `tx`
    Async {
        tx: async_channel::Sender<SpawnedActor>,
    },
}

pub(crate) struct SpawnedActor {
    pub(crate) doc_id: DocumentId,
    pub(crate) actor_id: DocumentActorId,
    pub(crate) inner: Arc<Mutex<DocActorInner>>,
    pub(crate) rx_tasks: async_channel::Receiver<ActorTask>,
    pub(crate) init_results: DocActorResult,
}
