mod ephemeral_message;
pub(crate) use ephemeral_message::EphemeralMessage;
mod ephemeral_session_id;
pub(crate) use ephemeral_session_id::EphemeralSessionId;
mod ephemeral_session;
pub(crate) use ephemeral_session::EphemeralSession;

pub(crate) struct OutgoingSessionDetails {
    pub(crate) counter: u64,
    pub(crate) session_id: EphemeralSessionId,
}
