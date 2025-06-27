use samod_core::{
    actors::{HubToDocMsg, document::io::DocumentIoResult},
    io::IoResult,
};

#[derive(Debug)]
pub(crate) enum ActorTask {
    HandleMessage(HubToDocMsg),
    IoComplete(IoResult<DocumentIoResult>),
}
