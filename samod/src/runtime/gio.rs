use std::pin::Pin;

use crate::runtime::JoinError;
use futures::FutureExt;

#[derive(Clone, Debug)]
pub struct GioRuntime;

impl GioRuntime {
    pub fn new() -> Self {
        Self
    }
}

impl Default for GioRuntime {
    fn default() -> Self {
        Self::new()
    }
}

impl crate::runtime::RuntimeHandle for GioRuntime {
    type JoinErr = GlibJoinError;

    type JoinFuture<O: Send + 'static> = GlibJoinHandle<O>;

    fn spawn<O, F>(&self, f: F) -> Self::JoinFuture<O>
    where
        O: Send + 'static,
        F: Future<Output = O> + Send + 'static,
    {
        GlibJoinHandle(glib::spawn_future(f))
    }
}

pub struct GlibJoinError(glib::JoinError);

impl std::fmt::Display for GlibJoinError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "GlibJoinError: {}", self.0)
    }
}

impl std::fmt::Debug for GlibJoinError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "GlibJoinError: {}", self.0)
    }
}

impl std::error::Error for GlibJoinError {}

impl JoinError for GlibJoinError {
    fn is_panic(&self) -> bool {
        false
    }

    fn into_panic(self) -> Box<dyn std::any::Any + Send + 'static> {
        panic!("not a panic error")
    }
}

pub struct GlibJoinHandle<O>(glib::JoinHandle<O>);

impl<O: 'static> Future for GlibJoinHandle<O> {
    type Output = Result<O, GlibJoinError>;

    fn poll(
        mut self: Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Self::Output> {
        match self.0.poll_unpin(cx) {
            std::task::Poll::Ready(Ok(value)) => std::task::Poll::Ready(Ok(value)),
            std::task::Poll::Ready(Err(err)) => std::task::Poll::Ready(Err(GlibJoinError(err))),
            std::task::Poll::Pending => std::task::Poll::Pending,
        }
    }
}
