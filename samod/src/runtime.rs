use std::any::Any;

use futures::Future;

#[cfg(feature = "gio")]
pub mod gio;
#[cfg(feature = "tokio")]
pub mod tokio;

pub trait RuntimeHandle: Clone + 'static {
    type JoinErr: JoinError + std::error::Error;
    type JoinFuture<O: Send + 'static>: Future<Output = Result<O, Self::JoinErr>> + Unpin;

    fn spawn<O, F>(&self, f: F) -> Self::JoinFuture<O>
    where
        O: Send + 'static,
        F: Future<Output = O> + Send + 'static;
}

pub trait JoinError {
    fn is_panic(&self) -> bool;
    fn into_panic(self) -> Box<dyn Any + Send + 'static>;
}
