//! WASM bindings for Samod
//!
//! This module provides JavaScript-friendly bindings for using Samod in web browsers.
//! All the main functionality of the Rust API is exposed through WebAssembly bindings.

use std::str::FromStr;

use automerge::{Automerge, transaction::Transactable};
use futures::{StreamExt, channel::oneshot};
use js_sys::Promise;
use samod_core::DocumentId;
use wasm_bindgen::prelude::*;
use wasm_bindgen_futures::future_to_promise;

use crate::{ConnDirection, ConnFinishedReason, DocHandle, Repo, storage::IndexedDbStorage};

/// JavaScript error type for WASM bindings
#[wasm_bindgen]
pub struct WasmError {
    message: String,
}

#[wasm_bindgen]
impl WasmError {
    #[wasm_bindgen(getter)]
    pub fn message(&self) -> String {
        self.message.clone()
    }
}

impl From<String> for WasmError {
    fn from(message: String) -> Self {
        Self { message }
    }
}

impl From<&str> for WasmError {
    fn from(message: &str) -> Self {
        Self {
            message: message.to_string(),
        }
    }
}

impl From<automerge::AutomergeError> for WasmError {
    fn from(error: automerge::AutomergeError) -> Self {
        Self {
            message: format!("Automerge error: {}", error),
        }
    }
}

/// WASM wrapper around a Samod repository
#[wasm_bindgen]
pub struct WasmRepo {
    inner: Repo,
}

#[wasm_bindgen]
impl WasmRepo {
    /// Create a new WASM repository with IndexedDB storage
    #[wasm_bindgen(constructor)]
    pub fn new() -> Promise {
        // Initialize WASM tracing to forward Rust logs to browser console
        console_error_panic_hook::set_once();
        tracing_wasm::set_as_global_default();

        future_to_promise(async {
            tracing::info!("WASM: Initializing Samod repository with IndexedDB storage");

            // Always use fixed database and store names for consistency
            let storage = IndexedDbStorage::with_names("samod_db", "documents");
            let repo = Repo::build_wasm()
                .with_storage(storage.clone())
                .load_local()
                .await;

            // Create a WasmRepo instance
            let wasm_repo = WasmRepo { inner: repo };

            // Restore documents from storage
            if let Err(e) = wasm_repo.restore_documents_from_storage(storage).await {
                tracing::warn!("Failed to restore some documents from storage: {}", e);
            }

            tracing::info!("WASM: Repository initialized and documents restored");
            Ok(JsValue::from(wasm_repo))
        })
    }

    /// Create a new document with optional initial content
    #[wasm_bindgen(js_name = "createDocument")]
    pub fn create_document(&self, initial_content: JsValue) -> Promise {
        let repo = self.inner.clone();

        future_to_promise(async move {
            let initial_doc = if !initial_content.is_undefined() {
                let mut doc = Automerge::new();
                populate_doc_from_js_value(&mut doc, &initial_content).map_err(|e| {
                    WasmError::from(format!(
                        "Failed to populate document from initial content: {}",
                        e
                    ))
                })?;
                doc
            } else {
                Automerge::new()
            };

            match repo.create(initial_doc).await {
                Ok(doc_handle) => Ok(JsValue::from(WasmDocHandle::new(doc_handle))),
                Err(_) => Err(JsValue::from(WasmError::from("Failed to create document"))),
            }
        })
    }

    /// Find an existing document by ID
    #[wasm_bindgen(js_name = "findDocument")]
    pub fn find_document(&self, document_id: &str) -> Promise {
        let repo = self.inner.clone();
        let doc_id = document_id.to_string();

        future_to_promise(async move {
            let document_id = match DocumentId::from_str(&doc_id) {
                Ok(id) => id,
                Err(_) => return Err(JsValue::from(WasmError::from("Invalid document ID format"))),
            };

            match repo.find(document_id).await {
                Ok(Some(doc_handle)) => Ok(JsValue::from(WasmDocHandle::new(doc_handle))),
                Ok(None) => Ok(JsValue::NULL),
                Err(_) => Err(JsValue::from(WasmError::from("Failed to find document"))),
            }
        })
    }

    /// Get the peer ID of this repository
    #[wasm_bindgen(js_name = "peerId")]
    pub fn peer_id(&self) -> String {
        self.inner.peer_id().to_string()
    }

    /// Stop the repository and flush all data to storage
    #[wasm_bindgen]
    pub fn stop(&self) -> Promise {
        let repo = self.inner.clone();

        future_to_promise(async move {
            repo.stop().await;
            Ok(JsValue::UNDEFINED)
        })
    }

    /// List all document IDs that are stored locally
    #[wasm_bindgen(js_name = "listDocuments")]
    pub fn list_documents(&self) -> Result<js_sys::Array, WasmError> {
        match self.inner.list_documents() {
            Ok(doc_ids) => {
                let js_array = js_sys::Array::new();
                for doc_id in doc_ids {
                    js_array.push(&JsValue::from_str(&doc_id.to_string()));
                }
                Ok(js_array)
            }
            Err(_) => Err(WasmError::from("Failed to list documents")),
        }
    }

    /// Connect to a WebSocket server for real-time synchronization
    /// Returns a handle that can be used to close the connection
    #[wasm_bindgen(js_name = "connectWebSocket")]
    pub fn connect_websocket(&self, url: &str) -> Result<WasmWebSocketHandle, WasmError> {
        #[cfg(feature = "wasm")]
        {
            let repo = self.inner.clone();
            let url = url.to_string();

            web_sys::console::log_1(
                &format!("WASM: Creating WebSocket connection handle for {}", url).into(),
            );

            // Create channels to signal close and receive finish notification
            let (close_tx, close_rx) = oneshot::channel::<()>();
            let (finish_tx, finish_rx) = oneshot::channel::<String>();

            // Spawn the connection task
            wasm_bindgen_futures::spawn_local(async move {
                web_sys::console::log_1(
                    &format!("WASM: Starting connection task for {}", url).into(),
                );

                // Create the websocket connection
                match crate::websocket::WasmWebSocket::connect(&url).await {
                    Ok(ws) => {
                        web_sys::console::log_1(
                            &"WASM: WebSocket connected, splitting into sink/stream".into(),
                        );
                        let (sink, stream) = StreamExt::split(ws);

                        web_sys::console::log_1(&"WASM: About to call connect_with_close_signal - protocol handshake should start".into());

                        // Run the connection with close signal
                        let result = repo
                            .connect_with_close_signal(
                                stream,
                                sink,
                                ConnDirection::Outgoing,
                                close_rx,
                            )
                            .await;

                        web_sys::console::log_1(
                            &format!("WASM: Connection ended with: {:?}", result).into(),
                        );
                        let reason_str = match result {
                            ConnFinishedReason::WeDisconnected => {
                                web_sys::console::log_1(
                                    &"WASM: Normal close initiated by client".into(),
                                );
                                "we_disconnected"
                            }
                            ConnFinishedReason::TheyDisconnected => {
                                web_sys::console::log_1(
                                    &"WASM: Connection closed by server".into(),
                                );
                                "server_disconnected"
                            }
                            ConnFinishedReason::ErrorReceiving(_) => {
                                web_sys::console::log_1(
                                    &"WASM: Connection ended due to receive error".into(),
                                );
                                "error_receiving"
                            }
                            ConnFinishedReason::ErrorSending(_) => {
                                web_sys::console::log_1(
                                    &"WASM: Connection ended due to send error".into(),
                                );
                                "error_sending"
                            }
                            ConnFinishedReason::Error(_) => {
                                web_sys::console::log_1(
                                    &"WASM: Connection ended due to generic error".into(),
                                );
                                "error"
                            }
                            ConnFinishedReason::Shutdown => {
                                web_sys::console::log_1(
                                    &"WASM: Connection ended due to shutdown".into(),
                                );
                                "shutdown"
                            }
                        };

                        // Notify about the connection ending
                        let _ = finish_tx.send(reason_str.to_string());
                    }
                    Err(e) => {
                        web_sys::console::error_1(
                            &format!("WASM: WebSocket connection failed: {}", e).into(),
                        );
                        // Notify about the connection failure
                        let _ = finish_tx.send("connection_failed".to_string());
                    }
                }
            });

            web_sys::console::log_1(&"WASM: Connection task spawned, returning handle".into());

            Ok(WasmWebSocketHandle {
                close_sender: Some(close_tx),
                finish_receiver: Some(finish_rx),
            })
        }
    }

    /// Connect to a WebSocket server and wait for completion (legacy API)
    #[wasm_bindgen(js_name = "connectWebSocketAsync")]
    pub fn connect_websocket_async(&self, url: &str) -> Promise {
        let repo = self.inner.clone();
        let url = url.to_string();

        future_to_promise(async move {
            #[cfg(feature = "wasm")]
            {
                let result = repo
                    .connect_wasm_websocket(&url, ConnDirection::Outgoing)
                    .await;
                match result {
                    ConnFinishedReason::Shutdown => Ok(JsValue::from_str("shutdown")),
                    ConnFinishedReason::TheyDisconnected => {
                        Ok(JsValue::from_str("they_disconnected"))
                    }
                    ConnFinishedReason::WeDisconnected => Ok(JsValue::from_str("we_disconnected")),
                    ConnFinishedReason::ErrorReceiving(err) => Err(JsValue::from(WasmError::from(
                        format!("Error receiving: {}", err),
                    ))),
                    ConnFinishedReason::ErrorSending(err) => Err(JsValue::from(WasmError::from(
                        format!("Error sending: {}", err),
                    ))),
                    ConnFinishedReason::Error(err) => Err(JsValue::from(WasmError::from(err))),
                }
            }
        })
    }
}

impl WasmRepo {
    /// Restore documents from IndexedDB storage
    /// This scans the storage for all document IDs and pre-loads them
    async fn restore_documents_from_storage(
        &self,
        storage: IndexedDbStorage,
    ) -> Result<(), String> {
        use crate::storage::LocalStorage;
        use samod_core::{DocumentId, StorageKey};
        use std::collections::HashSet;
        use std::str::FromStr;

        tracing::info!("WASM: Starting document restoration from storage");

        // Create a prefix to scan for all documents
        // Documents are stored with keys like "{doc_id}/snapshot/{hash}"
        // We want to find all unique document IDs
        let empty_prefix = StorageKey::from_parts(Vec::<String>::new())
            .map_err(|e| format!("Failed to create empty prefix: {}", e))?;

        // Load all keys that match our prefix (which is everything)
        let all_keys = storage.load_range(empty_prefix).await;

        // Extract unique document IDs from the keys
        let mut document_ids = HashSet::new();
        for (key, _) in all_keys {
            // Get the first component of the key, which should be the document ID
            let parts: Vec<String> = key.into_iter().map(|key| key.to_string()).collect();
            if !parts.is_empty() {
                if let Ok(doc_id) = DocumentId::from_str(&parts[0]) {
                    document_ids.insert(doc_id);
                }
            }
        }

        tracing::info!("WASM: Found {} documents in storage", document_ids.len());

        // Pre-load each document by calling find()
        // This will create the document actors and load them into memory
        for doc_id in document_ids {
            tracing::debug!("WASM: Restoring document {}", doc_id);
            match self.inner.find(doc_id.clone()).await {
                Ok(Some(_)) => {
                    tracing::debug!("WASM: Successfully restored document {}", doc_id);
                }
                Ok(None) => {
                    tracing::warn!(
                        "WASM: Document {} not found despite being in storage",
                        doc_id
                    );
                }
                Err(e) => {
                    tracing::error!("WASM: Failed to restore document {}: {:?}", doc_id, e);
                }
            }
        }

        tracing::info!("WASM: Document restoration complete");
        Ok(())
    }
}

/// Handle for managing WebSocket connections in WASM
#[wasm_bindgen]
pub struct WasmWebSocketHandle {
    close_sender: Option<oneshot::Sender<()>>,
    finish_receiver: Option<oneshot::Receiver<String>>,
}

#[wasm_bindgen]
impl WasmWebSocketHandle {
    /// Close the WebSocket connection
    #[wasm_bindgen]
    pub fn close(&mut self) {
        if let Some(sender) = self.close_sender.take() {
            let _ = sender.send(());
        }
    }

    /// Check if the connection has ended and get the reason
    /// Returns a Promise that resolves with the disconnect reason or null if still connected
    #[wasm_bindgen(js_name = "waitForDisconnect")]
    pub fn wait_for_disconnect(&mut self) -> Promise {
        if let Some(receiver) = self.finish_receiver.take() {
            future_to_promise(async move {
                match receiver.await {
                    Ok(reason) => Ok(JsValue::from_str(&reason)),
                    Err(_) => Ok(JsValue::NULL),
                }
            })
        } else {
            future_to_promise(async { Ok(JsValue::NULL) })
        }
    }
}

/// WASM wrapper around a Samod document handle
#[wasm_bindgen]
pub struct WasmDocHandle {
    inner: DocHandle,
}

impl WasmDocHandle {
    fn new(inner: DocHandle) -> Self {
        Self { inner }
    }
}

#[wasm_bindgen]
impl WasmDocHandle {
    /// Get the document ID as a string
    #[wasm_bindgen(js_name = "documentId")]
    pub fn document_id(&self) -> String {
        self.inner.document_id().to_string()
    }

    /// Get the document URL (compatible with automerge-repo)
    #[wasm_bindgen]
    pub fn url(&self) -> String {
        self.inner.url().to_string()
    }

    /// Get the current state of the document as a JavaScript object
    #[wasm_bindgen(js_name = "getDocument")]
    pub fn get_document(&self) -> Result<JsValue, WasmError> {
        let mut result = None;
        self.inner
            .with_document(|doc| {
                // Convert the automerge document to a JS-friendly format
                let js_doc = automerge_to_js_value(doc)?;
                result = Some(js_doc);
                Ok::<_, automerge::AutomergeError>(())
            })
            .map_err(|e| WasmError::from(format!("Failed to access document: {}", e)))?;

        Ok(result.unwrap_or(JsValue::NULL))
    }
}

/// Helper function to convert a JavaScript value to Automerge operations
fn js_value_to_automerge<T: Transactable, K: Into<automerge::Prop> + Clone>(
    tx: &mut T,
    obj: automerge::ObjId,
    key: K,
    js_value: &JsValue,
) -> Result<(), automerge::AutomergeError> {
    use automerge::ScalarValue;
    use js_sys::{Array, Object, Uint8Array};
    use wasm_bindgen::JsCast;

    if js_value.is_null() {
        tx.put(obj, key, ScalarValue::Null)?;
    } else if js_value.is_undefined() {
        // Skip undefined values
        return Ok(());
    } else if let Some(bool_val) = js_value.as_bool() {
        tx.put(obj, key, bool_val)?;
    } else if let Some(num_val) = js_value.as_f64() {
        if num_val.fract() == 0.0 && num_val >= i64::MIN as f64 && num_val <= i64::MAX as f64 {
            tx.put(obj, key, num_val as i64)?;
        } else {
            tx.put(obj, key, num_val)?;
        }
    } else if let Some(str_val) = js_value.as_string() {
        tx.put(obj, key, str_val)?;
    } else if js_value.is_instance_of::<Uint8Array>() {
        // Handle Uint8Array (binary data)
        let uint8_array = js_value.dyn_ref::<Uint8Array>().unwrap();
        let bytes = uint8_array.to_vec();
        tx.put(obj, key, ScalarValue::Bytes(bytes))?;
    } else if Array::is_array(js_value) {
        let array = Array::from(js_value);
        let list_obj = tx.put_object(obj, key.clone(), automerge::ObjType::List)?;

        for i in 0..array.length() {
            let item = array.get(i);
            js_value_to_automerge(tx, list_obj.clone(), i as usize, &item)?;
        }
    } else if js_value.is_object() {
        let obj_js = Object::from(js_value.clone());
        let map_obj = tx.put_object(obj, key.clone(), automerge::ObjType::Map)?;

        let keys = Object::keys(&obj_js);
        for i in 0..keys.length() {
            let key_js = keys.get(i);
            if let Some(key_str) = key_js.as_string() {
                let value = js_sys::Reflect::get(&obj_js, &key_js)
                    .map_err(|_| automerge::AutomergeError::InvalidOp(automerge::ObjType::Map))?;
                js_value_to_automerge(tx, map_obj.clone(), &key_str, &value)?;
            } else {
                return Err(automerge::AutomergeError::InvalidOp(
                    automerge::ObjType::Map,
                ));
            }
        }
    } else {
        // Handle unsupported JavaScript types
        return Err(automerge::AutomergeError::InvalidOp(
            automerge::ObjType::Map,
        ));
    }

    Ok(())
}

/// Helper function to recursively populate an entire document from a JavaScript object
fn populate_doc_from_js_value(
    doc: &mut Automerge,
    js_value: &JsValue,
) -> Result<(), automerge::AutomergeError> {
    use js_sys::Object;

    if js_value.is_object() && !js_value.is_null() {
        let obj_js = Object::from(js_value.clone());
        let keys = Object::keys(&obj_js);

        doc.transact(|tx| {
            for i in 0..keys.length() {
                let key_js = keys.get(i);
                if let Some(key_str) = key_js.as_string() {
                    let value = js_sys::Reflect::get(&obj_js, &key_js).map_err(|_| {
                        automerge::AutomergeError::InvalidOp(automerge::ObjType::Map)
                    })?;
                    js_value_to_automerge(tx, automerge::ROOT, &key_str, &value)?;
                }
            }
            Ok(())
        })
        .map_err(
            |_: automerge::transaction::Failure<automerge::AutomergeError>| {
                automerge::AutomergeError::InvalidOp(automerge::ObjType::Map)
            },
        )?;
    }

    Ok(())
}

/// Helper function to convert an Automerge document to a JavaScript value
fn automerge_to_js_value(doc: &Automerge) -> Result<JsValue, automerge::AutomergeError> {
    automerge_obj_to_js_value(doc, automerge::ROOT)
}

/// Helper function to recursively convert an Automerge object to a JavaScript value
fn automerge_obj_to_js_value(
    doc: &Automerge,
    obj: automerge::ObjId,
) -> Result<JsValue, automerge::AutomergeError> {
    use automerge::{ObjType, ReadDoc};
    use js_sys::{Array, Object};

    match doc.object_type(&obj)? {
        ObjType::Map => {
            let js_obj = Object::new();

            // Get all properties from the map object
            for item in doc.map_range(&obj, ..) {
                match item.value {
                    automerge::ValueRef::Object(_) => {
                        // For object values, we need to handle them using the object ID
                        // which is available in the item context
                        let obj_id = item.id();
                        let prop = item.key;
                        let result = automerge_obj_to_js_value(doc, obj_id)?;
                        js_sys::Reflect::set(&js_obj, &JsValue::from_str(&prop), &result)
                            .map_err(|_| automerge::AutomergeError::InvalidOp(ObjType::Map))?;
                    }
                    value => {
                        let prop = item.key;
                        let result = automerge_value_to_js_value(doc, value.into())?;
                        js_sys::Reflect::set(&js_obj, &JsValue::from_str(&prop), &result)
                            .map_err(|_| automerge::AutomergeError::InvalidOp(ObjType::Map))?;
                    }
                };
            }

            Ok(js_obj.into())
        }
        ObjType::List => {
            let js_array = Array::new();
            let len = doc.length(&obj);

            // Get all items from the list object
            for i in 0..len {
                if let Ok(Some((value, _))) = doc.get(&obj, i) {
                    let js_value = automerge_value_to_js_value(doc, value)?;
                    js_array.push(&js_value);
                }
            }

            Ok(js_array.into())
        }
        ObjType::Text => {
            // Convert text object to string
            let text = doc.text(&obj)?;
            Ok(JsValue::from_str(&text))
        }
        ObjType::Table => {
            // Tables are handled similar to maps
            let js_obj = Object::new();

            // Get all properties from the table object
            for item in doc.map_range(&obj, ..) {
                match item.value {
                    automerge::ValueRef::Object(_) => {
                        let obj_id = item.id();
                        let prop = item.key;
                        let result = automerge_obj_to_js_value(doc, obj_id)?;
                        js_sys::Reflect::set(&js_obj, &JsValue::from_str(&prop), &result)
                            .map_err(|_| automerge::AutomergeError::InvalidOp(ObjType::Table))?;
                    }
                    value => {
                        let prop = item.key;
                        let result = automerge_value_to_js_value(doc, value.into())?;
                        js_sys::Reflect::set(&js_obj, &JsValue::from_str(&prop), &result)
                            .map_err(|_| automerge::AutomergeError::InvalidOp(ObjType::Table))?;
                    }
                };
            }

            Ok(js_obj.into())
        }
    }
}

/// Helper function to convert an Automerge value to a JavaScript value
fn automerge_value_to_js_value(
    _doc: &Automerge,
    value: automerge::Value<'_>,
) -> Result<JsValue, automerge::AutomergeError> {
    use automerge::Value;

    match value {
        Value::Scalar(scalar_value) => match scalar_value.as_ref() {
            automerge::ScalarValue::Str(s) => Ok(JsValue::from_str(s.as_str())),
            automerge::ScalarValue::Int(i) => Ok(JsValue::from(*i as f64)),
            automerge::ScalarValue::Uint(u) => Ok(JsValue::from(*u as f64)),
            automerge::ScalarValue::F64(f) => Ok(JsValue::from(*f)),
            automerge::ScalarValue::Boolean(b) => Ok(JsValue::from(*b)),
            automerge::ScalarValue::Bytes(bytes) => Ok(js_sys::Uint8Array::from(&bytes[..]).into()),
            automerge::ScalarValue::Null => Ok(JsValue::NULL),
            automerge::ScalarValue::Timestamp(t) => Ok(JsValue::from(*t as f64)),
            automerge::ScalarValue::Counter(c) => Ok(JsValue::from(format!("{}", c))),
            _ => Ok(JsValue::UNDEFINED),
        },
        Value::Object(obj_type) => Err(automerge::AutomergeError::InvalidOp(obj_type.to_owned())),
    }
}
