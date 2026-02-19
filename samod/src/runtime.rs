use std::{pin::Pin, time::Duration};

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
pub trait RuntimeHandle: 'static {
    /// Spawn a task to be run in the background
    fn spawn(&self, f: Pin<Box<dyn Future<Output = ()> + Send + 'static>>);

    /// Return a future that completes after `duration` has elapsed.
    ///
    /// The returned future must be `Send + 'static` to match the requirements
    /// of [`RuntimeHandle::spawn`].
    fn sleep(&self, duration: Duration) -> Pin<Box<dyn Future<Output = ()> + Send + 'static>>;
}

/// An abstraction over the asynchronous runtime the repo is running on
///
/// When a [`Repo`](crate::Repo) starts up it spawns a number of tasks which run
/// until the repo is shutdown. These tasks do things like handle IO using
/// [`Storage`](crate::Storage) or pass messages between different document
/// threads and the central control loop of the repo. [`LocalRuntimeHandle`]
/// represents this ability to spawn tasks.
///
/// The difference between this trait and the [`RuntimeHandle`] trait is that
/// the `LocalRuntimeHandle` does not have a `Send` or 'static bound, enabling
/// it to be used with runtimes that don't require this. See the [module level
/// documentation on runtimes](../index.html#runtimes) for more details.
pub trait LocalRuntimeHandle {
    /// Spawn a task to be run in the background
    fn spawn(&self, f: Pin<Box<dyn Future<Output = ()>>>);

    /// Return a future that completes after `duration` has elapsed.
    fn sleep(&self, duration: Duration) -> Pin<Box<dyn Future<Output = ()>>>;
}
