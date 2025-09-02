use std::pin::Pin;

use crate::runtime::RuntimeHandle;
use std::future::Future;

pub struct WasmRuntime;

impl WasmRuntime {
    pub fn new() -> Self {
        Self
    }
}

impl Default for WasmRuntime {
    fn default() -> Self {
        Self::new()
    }
}

impl RuntimeHandle for WasmRuntime {
    fn spawn(&self, f: Pin<Box<dyn Future<Output = ()> + Send + 'static>>) {
        wasm_bindgen_futures::spawn_local(f);
    }
}
