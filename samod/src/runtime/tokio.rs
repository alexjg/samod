use std::pin::Pin;

use crate::runtime::RuntimeHandle;

impl RuntimeHandle for tokio::runtime::Handle {
    fn spawn(&self, f: Pin<Box<dyn Future<Output = ()> + Send + 'static>>) {
        self.spawn(f);
    }
}
