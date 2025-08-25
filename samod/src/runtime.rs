//! An abstraction over async runtimes

use std::any::Any;

use futures::Future;

#[cfg(feature = "gio")]
pub mod gio;
pub mod localpool;
#[cfg(feature = "tokio")]
mod tokio;

/// An abstraction over the asynchronous runtime the repo is running on
///
/// When a [`Repo`](crate::Repo) starts up it spawns a number of tasks which run
/// until the repo is shutdown. These tasks do things like handle IO using
/// [`Storage`](crate::Storage) or pass messages between different document
/// threads and the central control loop of the repo. [`RuntimeHandle`]
/// represents this ability to spawn tasks.
pub trait RuntimeHandle: Clone + 'static {
    type JoinErr: JoinError + std::error::Error;
    type JoinFuture<O: Send + 'static>: Future<Output = Result<O, Self::JoinErr>> + Unpin;

    /// Spawn a task to be run in the background
    fn spawn<O, F>(&self, f: F) -> Self::JoinFuture<O>
    where
        O: Send + 'static,
        F: Future<Output = O> + Send + 'static;
}

pub trait JoinError {
    fn is_panic(&self) -> bool;
    fn into_panic(self) -> Box<dyn Any + Send + 'static>;
}
