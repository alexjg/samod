use std::sync::{Arc, Mutex};

use crate::{DocActorInner, DocHandle};

pub(crate) struct ActorHandle {
    pub(crate) inner: Arc<Mutex<DocActorInner>>,
    pub(crate) doc: DocHandle,
}
