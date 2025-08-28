use std::sync::{Arc, Mutex};

use crate::{ActorTask, DocActorInner, DocHandle};

pub(crate) struct ActorHandle {
    #[allow(dead_code)]
    pub(crate) inner: Arc<Mutex<DocActorInner>>,
    pub(crate) tx: async_channel::Sender<ActorTask>,
    pub(crate) doc: DocHandle,
}
