use wasm_bindgen::prelude::*;

thread_local! {
    static TIME_PROVIDER: std::cell::RefCell<Option<js_sys::Function>> = std::cell::RefCell::new(None);
}

#[wasm_bindgen]
pub fn set_time_provider(callback: js_sys::Function) {
    TIME_PROVIDER.with(|provider| {
        *provider.borrow_mut() = Some(callback);
    });
}

pub(crate) fn get_external_time() -> Option<u128> {
    TIME_PROVIDER.with(|provider| {
        let provider_ref = provider.borrow();
        if let Some(callback) = provider_ref.as_ref() {
            let result = callback.call0(&JsValue::NULL).ok()?;
            Some(result.as_f64()? as u128)
        } else {
            None
        }
    })
}
