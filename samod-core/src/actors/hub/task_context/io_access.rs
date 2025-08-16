use std::task::Poll;

use futures::{Future, FutureExt, channel::oneshot};

use crate::{
    ConnectionId,
    actors::{
        driver::{ActorIo, DriverOutput},
        hub::{
            Hub,
            connection::Connection,
            io::{HubIoAction, HubIoResult},
            run::HubOutput,
        },
    },
};

#[derive(Clone)]
pub(crate) struct IoAccess {
    io: ActorIo<Hub>,
}

impl IoAccess {
    pub(crate) fn new(io: ActorIo<Hub>) -> Self {
        IoAccess { io }
    }

    pub(crate) fn send(&self, conn: &Connection, msg: Vec<u8>) {
        tracing::trace!(conn_id=?conn.id(), remote_peer_id=?conn.remote_peer_id(), num_bytes=msg.len(), "sending message");
        self.io.fire_and_forget_io(HubIoAction::Send {
            connection_id: conn.id(),
            msg: msg.clone(),
        });
    }

    pub(crate) fn disconnect(&self, connection_id: ConnectionId) {
        let _ = self.dispatch_task(HubIoAction::Disconnect { connection_id });
    }

    fn dispatch_task(&self, task: HubIoAction) -> impl Future<Output = HubIoResult> + 'static {
        self.io.perform_io(task)
    }

    pub(crate) fn emit_event(&self, event: HubOutput) {
        self.io.emit_event(event);
    }
}

struct TaskFuture<T>(oneshot::Receiver<T>);

impl<T> Future for TaskFuture<T> {
    type Output = T;

    fn poll(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Self::Output> {
        match self.0.poll_unpin(cx) {
            Poll::Ready(Ok(r)) => Poll::Ready(r),
            Poll::Ready(Err(_)) => {
                // In this case the other end of the channel was dropped, but it's held by the driver
                // of the samod instance, so this means the whole samod was dropped, so we don't care
                // about this error
                Poll::Pending
            }
            Poll::Pending => Poll::Pending,
        }
    }
}
