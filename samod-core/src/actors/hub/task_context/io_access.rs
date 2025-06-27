use std::task::Poll;

use futures::{Future, FutureExt, channel::oneshot};

use crate::{
    ConnectionId,
    actors::{
        driver::ActorIo,
        hub::{
            Hub,
            connection::Connection,
            io::{HubIoAction, HubIoResult},
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

    pub(crate) fn disconnect(
        &self,
        connection_id: ConnectionId,
    ) -> impl Future<Output = ()> + 'static {
        let result = self.dispatch_task(HubIoAction::Disconnect { connection_id });
        async move {
            let result = result.await;
            match result {
                HubIoResult::Disconnect => (),
                _ => panic!("Expected IoResultPayload::Disconnect, got {result:?}"),
            }
        }
    }

    fn dispatch_task(&self, task: HubIoAction) -> impl Future<Output = HubIoResult> + 'static {
        self.io.perform_io(task)
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
