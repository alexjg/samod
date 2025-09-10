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

impl<T> Clone for UnboundedSender<T> {
    fn clone(&self) -> Self {
        Self(self.0.clone())
    }
}

pub(crate) struct UnboundedReceiver<T>(async_channel::Receiver<T>);

impl<T> UnboundedReceiver<T> {
    #[cfg(feature = "threadpool")]
    pub(crate) fn recv_blocking(&self) -> Result<T, async_channel::RecvError> {
        self.0.recv_blocking()
    }

    pub(crate) async fn recv(&self) -> Result<T, async_channel::RecvError> {
        self.0.recv().await
    }
}
