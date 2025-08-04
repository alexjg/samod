use std::{
    collections::HashMap,
    fmt::Debug,
    sync::{Arc, Mutex},
};

use futures::channel::{mpsc, oneshot};

use crate::{
    UnixTimestamp,
    io::{IoResult, IoTask, IoTaskId},
};

use super::executor::LocalExecutor;

pub(crate) trait Actor {
    type IoTaskAction: Debug;
    type IoResult: Debug + 'static;
    type StepResults;
    type Output: Debug;
    type Input: Debug;
    type Complete;

    fn finish_step(
        outputs: Vec<Self::Output>,
        new_io_tasks: Vec<IoTask<Self::IoTaskAction>>,
    ) -> Self::StepResults;
}

pub(crate) struct Driver<A: Actor> {
    now: Arc<Mutex<UnixTimestamp>>,
    io_tasks: HashMap<IoTaskId, oneshot::Sender<A::IoResult>>,
    rx_output: mpsc::UnboundedReceiver<DriverOutput<A>>,
    tx_input: mpsc::UnboundedSender<A::Input>,
    executor: LocalExecutor<A::Complete>,
}

pub(crate) struct SpawnArgs<A: Actor> {
    pub(crate) now: Arc<Mutex<UnixTimestamp>>,
    pub(crate) io: ActorIo<A>,
    pub(crate) rx_input: mpsc::UnboundedReceiver<A::Input>,
}

pub(crate) enum DriverOutput<A: Actor> {
    Io {
        task: A::IoTaskAction,
        reply: oneshot::Sender<A::IoResult>,
    },
    Output(A::Output),
}

impl<A: Actor> std::fmt::Debug for DriverOutput<A>
where
    A::Output: Debug,
{
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DriverOutput::Io { task, reply: _ } => write!(f, "Io({task:?})"),
            DriverOutput::Output(output) => write!(f, "Output({output:?})"),
        }
    }
}

pub(crate) enum StepResult<A: Actor> {
    Suspend(A::StepResults),
    Complete {
        results: A::StepResults,
        complete: A::Complete,
    },
}

impl<A: Actor> Driver<A> {
    pub fn spawn<S: FnOnce(SpawnArgs<A>) -> F, F: Future<Output = A::Complete> + Send + 'static>(
        now: UnixTimestamp,
        spawn: S,
    ) -> Self {
        let now = Arc::new(Mutex::new(now));
        let (tx_output, rx_output) = mpsc::unbounded();
        let (tx_input, rx_input) = mpsc::unbounded();
        let future = spawn(SpawnArgs {
            io: ActorIo { tx_output },
            rx_input,
            now: now.clone(),
        });
        let executor = LocalExecutor::spawn(future);
        Self {
            now,
            io_tasks: HashMap::new(),
            rx_output,
            tx_input,
            executor,
        }
    }

    pub fn handle_input(&mut self, now: UnixTimestamp, input: A::Input) {
        *self.now.lock().unwrap() = now;
        let _ = self.tx_input.unbounded_send(input);
    }

    pub fn handle_io_complete(&mut self, now: UnixTimestamp, result: IoResult<A::IoResult>) {
        *self.now.lock().unwrap() = now;
        if let Some(reply) = self.io_tasks.remove(&result.task_id) {
            let _ = reply.send(result.payload);
        } else {
            tracing::warn!(
                "Received IO result for unknown task ID: {:?}",
                result.task_id
            );
        }
    }

    pub fn step(&mut self, now: UnixTimestamp) -> StepResult<A> {
        *self.now.lock().unwrap() = now;
        let future_result = self.executor.run_until_stalled();
        let mut outputs = Vec::new();
        let mut new_io_tasks = Vec::new();
        while let Ok(Some(out)) = self.rx_output.try_next() {
            match out {
                DriverOutput::Io { task, reply } => {
                    let task_id = IoTaskId::new();
                    self.io_tasks.insert(task_id, reply);
                    new_io_tasks.push(IoTask {
                        task_id,
                        action: task,
                    });
                }
                DriverOutput::Output(output) => outputs.push(output),
            }
        }
        let step_results = A::finish_step(outputs, new_io_tasks);
        if let Some(result) = future_result {
            StepResult::Complete {
                results: step_results,
                complete: result,
            }
        } else {
            StepResult::Suspend(step_results)
        }
    }
}

pub(crate) struct ActorIo<A: Actor> {
    tx_output: mpsc::UnboundedSender<DriverOutput<A>>,
}

impl<A: Actor> std::clone::Clone for ActorIo<A> {
    fn clone(&self) -> Self {
        Self {
            tx_output: self.tx_output.clone(),
        }
    }
}

impl<A: Actor> ActorIo<A> {
    pub(crate) fn perform_io(
        &self,
        task: A::IoTaskAction,
    ) -> impl Future<Output = A::IoResult> + 'static {
        let (tx, rx) = oneshot::channel();
        let _ = self
            .tx_output
            .unbounded_send(DriverOutput::Io { task, reply: tx });
        async move { rx.await.expect("IO task reply channel closed") }
    }

    pub(crate) fn fire_and_forget_io(&self, task: A::IoTaskAction) {
        let _ = self.tx_output.unbounded_send(DriverOutput::Io {
            task,
            reply: oneshot::channel().0,
        });
    }

    pub(crate) fn emit_event(&self, event: A::Output) {
        let _ = self.tx_output.unbounded_send(DriverOutput::Output(event));
    }
}
