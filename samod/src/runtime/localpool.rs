use std::{pin::Pin, time::Duration};

use futures::{executor::LocalSpawner, task::LocalSpawnExt};

use crate::runtime::{LocalRuntimeHandle, RuntimeHandle};

#[derive(Clone)]
pub struct LocalPoolRuntime;

impl RuntimeHandle for LocalSpawner {
    fn spawn(&self, f: Pin<Box<dyn Future<Output = ()> + Send + 'static>>) {
        self.spawn_local(f).unwrap();
    }

    fn sleep(&self, duration: Duration) -> Pin<Box<dyn Future<Output = ()> + Send + 'static>> {
        // futures-timer uses a background thread-based timer wheel, which
        // works with any executor including futures::executor::LocalPool.
        Box::pin(futures_timer::Delay::new(duration))
    }
}

impl LocalRuntimeHandle for LocalSpawner {
    fn spawn(&self, f: Pin<Box<dyn Future<Output = ()>>>) {
        self.spawn_local(f).unwrap();
    }

    fn sleep(&self, duration: Duration) -> Pin<Box<dyn Future<Output = ()>>> {
        Box::pin(futures_timer::Delay::new(duration))
    }
}
