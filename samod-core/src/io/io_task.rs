use super::IoTaskId;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct IoTask<Action> {
    pub task_id: IoTaskId,
    pub action: Action,
}
