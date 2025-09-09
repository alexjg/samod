use std::pin::Pin;

use futures::{executor::LocalSpawner, task::LocalSpawnExt};

use crate::runtime::{LocalRuntimeHandle, RuntimeHandle};

#[derive(Clone)]
pub struct LocalPoolRuntime;

impl RuntimeHandle for LocalSpawner {
    fn spawn(&self, f: Pin<Box<dyn Future<Output = ()> + Send + 'static>>) {
        self.spawn_local(f).unwrap();
    }
}

impl LocalRuntimeHandle for LocalSpawner {
    fn spawn(&self, f: Pin<Box<dyn Future<Output = ()>>>) {
        self.spawn_local(f).unwrap();
    }
}
