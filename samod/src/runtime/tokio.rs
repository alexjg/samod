use std::{pin::Pin, time::Duration};

use crate::runtime::RuntimeHandle;

impl RuntimeHandle for tokio::runtime::Handle {
    fn spawn(&self, f: Pin<Box<dyn Future<Output = ()> + Send + 'static>>) {
        self.spawn(f);
    }

    fn sleep(&self, duration: Duration) -> Pin<Box<dyn Future<Output = ()> + Send + 'static>> {
        Box::pin(tokio::time::sleep(duration))
    }
}
