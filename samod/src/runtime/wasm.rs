use std::pin::Pin;
use std::future::Future;

use crate::runtime::{LocalRuntimeHandle, RuntimeHandle};

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

impl LocalRuntimeHandle for WasmRuntime {
    fn spawn(&self, f: Pin<Box<dyn Future<Output = ()>>>) {
        wasm_bindgen_futures::spawn_local(f);
    }
}
