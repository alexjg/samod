use std::pin::Pin;

/// A [`RuntimeHandle`](crate::runtime::RuntimeHandle) implementation which usese the `glib` crate to spawn tasks
///
/// This runtime will panic if used outside of a `glib` main loop context
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
    fn spawn(&self, f: Pin<Box<dyn Future<Output = ()> + Send + 'static>>) {
        glib::spawn_future(f);
    }
}
