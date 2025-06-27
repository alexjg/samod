use super::IoTaskId;

#[derive(Debug)]
pub struct IoResult<Payload> {
    pub task_id: IoTaskId,
    pub payload: Payload,
}
