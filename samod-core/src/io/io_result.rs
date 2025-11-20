use super::IoTaskId;

#[derive(Debug, Clone)]
pub struct IoResult<Payload> {
    pub task_id: IoTaskId,
    pub payload: Payload,
}
