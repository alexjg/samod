use std::sync::{Arc, Mutex};

use crate::{ActorTask, DocActorInner, DocHandle, unbounded::UnboundedSender};

pub(crate) struct ActorHandle {
    #[allow(dead_code)]
    pub(crate) inner: Arc<Mutex<DocActorInner>>,
    pub(crate) tx: UnboundedSender<ActorTask>,
    pub(crate) doc: DocHandle,
}
