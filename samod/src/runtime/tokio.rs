use std::any::Any;

use crate::runtime::{JoinError, RuntimeHandle};

use tokio::task::JoinError as TokioJoinError;

#[cfg_attr(docsrs, doc(cfg(feature = "tokio")))]
impl RuntimeHandle for tokio::runtime::Handle {
    type JoinErr = tokio::task::JoinError;
    type JoinFuture<O: Send + 'static> = tokio::task::JoinHandle<O>;

    fn spawn<O, F>(&self, f: F) -> Self::JoinFuture<O>
    where
        O: Send + 'static,
        F: Future<Output = O> + Send + 'static,
    {
        self.spawn(f)
    }
}

impl JoinError for TokioJoinError {
    fn is_panic(&self) -> bool {
        self.is_panic()
    }

    fn into_panic(self) -> Box<dyn Any + Send + 'static> {
        self.into_panic()
    }
}
