use std::sync::{Arc, Mutex};

use crate::{DocActorInner, DocHandle};

pub(crate) struct ActorHandle {
    #[allow(dead_code)]
    pub(crate) inner: Arc<Mutex<DocActorInner>>,
    pub(crate) doc: DocHandle,
}
