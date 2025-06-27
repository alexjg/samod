use automerge::ChangeHash;

use crate::actors::messages::DocToHubMsgPayload;

/// Internal output messages from the async actor runtime.
///
/// These messages are sent from the internal async `actor_run` function
/// back to the synchronous interface.
#[derive(Debug)]
pub(crate) enum ActorOutput {
    /// A message to be sent back to the hub actor
    Message(DocToHubMsgPayload),
    EphemeralMessage(Vec<u8>),
    DocChanged {
        new_heads: Vec<ChangeHash>,
    },
}
