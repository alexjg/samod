use std::{
    pin::Pin,
    task::{Context, Poll},
};

use futures::{Sink, Stream};

/// A wrapper around `async_channel::unbounded` which removes the `TrySendError::Full` error.
pub(crate) fn channel<T>() -> (UnboundedSender<T>, UnboundedReceiver<T>) {
    let (tx, rx) = async_channel::unbounded();
    (UnboundedSender(tx), UnboundedReceiver(rx))
}

pub(crate) struct UnboundedSender<T>(async_channel::Sender<T>);

impl<T> UnboundedSender<T> {
    pub(crate) fn unbounded_send(&self, msg: T) -> Result<(), T> {
        self.0.try_send(msg).map_err(|e| match e {
            async_channel::TrySendError::Full(_val) => {
                unreachable!("this is an unbounded channel")
            }
            async_channel::TrySendError::Closed(val) => val,
        })
    }
}

impl<T> Sink<T> for UnboundedSender<T> {
    type Error = ChanErr;

    fn poll_ready(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        // It's an unbounded channel, so we're always ready to send
        Poll::Ready(Ok(()))
    }

    fn start_send(self: Pin<&mut Self>, item: T) -> Result<(), Self::Error> {
        self.unbounded_send(item).map_err(|_e| ChanErr)
    }

    fn poll_flush(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }

    fn poll_close(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.0.close();
        Poll::Ready(Ok(()))
    }
}

#[derive(Debug)]
pub struct ChanErr;
impl std::fmt::Display for ChanErr {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Channel error")
    }
}
impl std::error::Error for ChanErr {}

impl<T> Clone for UnboundedSender<T> {
    fn clone(&self) -> Self {
        Self(self.0.clone())
    }
}

pub(crate) struct UnboundedReceiver<T>(async_channel::Receiver<T>);

impl<T> Unpin for UnboundedReceiver<T> {}

impl<T> UnboundedReceiver<T> {
    pub(crate) async fn recv(&self) -> Result<T, async_channel::RecvError> {
        self.0.recv().await
    }
}

impl<T> Stream for UnboundedReceiver<T> {
    type Item = T;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        // SAFETY: We never move the inner receiver
        unsafe { self.map_unchecked_mut(|s| &mut s.0) }.poll_next(cx)
    }
}
