use samod_core::{
    actors::{HubToDocMsg, document::io::DocumentIoResult},
    io::IoResult,
};

use crate::doc_actor_inner::DocActorInner;

pub(crate) enum ActorTask {
    HandleMessage(HubToDocMsg),
    IoComplete(IoResult<DocumentIoResult>),
    WithDocument(Box<dyn FnOnce(&mut DocActorInner) + Send + 'static>),
}

impl std::fmt::Debug for ActorTask {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ActorTask::HandleMessage(msg) => f.debug_tuple("HandleMessage").field(msg).finish(),
            ActorTask::IoComplete(result) => f.debug_tuple("IoComplete").field(result).finish(),
            ActorTask::WithDocument(_) => f.debug_tuple("WithDocument").finish(),
        }
    }
}
