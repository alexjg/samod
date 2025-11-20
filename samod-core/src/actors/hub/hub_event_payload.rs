use crate::io::IoResult;

use super::{HubInput, io::HubIoResult};

#[derive(Debug, Clone)]
pub(crate) enum HubEventPayload {
    // Some IO has completed
    IoComplete(IoResult<HubIoResult>),
    // Some other non IO event which should be handled by the actor loop
    Input(HubInput),
}
