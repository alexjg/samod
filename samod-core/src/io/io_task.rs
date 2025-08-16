use super::IoTaskId;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct IoTask<Action> {
    pub task_id: IoTaskId,
    pub action: Action,
}

impl<Action> IoTask<Action> {
    pub(crate) fn new(action: Action) -> Self {
        Self {
            task_id: IoTaskId::new(),
            action,
        }
    }

    pub(crate) fn map<T, F: FnOnce(Action) -> T>(self, f: F) -> IoTask<T> {
        IoTask {
            task_id: self.task_id,
            action: f(self.action),
        }
    }
}
