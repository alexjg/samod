use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};

use samod_core::{DocumentActorId, DocumentId, actors::document::DocActorResult};

use crate::{
    actor_task::ActorTask,
    doc_actor_inner::DocActorInner,
    unbounded::{UnboundedReceiver, UnboundedSender},
};

/// Enum representing the two possible ways of running document actors
pub(crate) enum DocRunner {
    /// Run the actors on a threadpool
    #[cfg(feature = "threadpool")]
    Threadpool(rayon::ThreadPool),
    /// Run the actors on an async task which is listening on the other end of `tx_spawn`
    Async {
        /// Channel to send newly spawned actors to the async runner
        tx_spawn: UnboundedSender<SpawnedActor>,
        /// Channels to send tasks to individual actors (keyed by actor ID)
        task_senders: HashMap<DocumentActorId, UnboundedSender<ActorTask>>,
    },
}

pub(crate) struct SpawnedActor {
    pub(crate) doc_id: DocumentId,
    pub(crate) actor_id: DocumentActorId,
    pub(crate) inner: Arc<Mutex<DocActorInner>>,
    pub(crate) rx_tasks: UnboundedReceiver<ActorTask>,
    pub(crate) init_results: DocActorResult,
}
