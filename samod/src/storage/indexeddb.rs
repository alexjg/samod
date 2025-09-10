use std::{collections::HashMap, future::Future};

use indexed_db_futures::{database::Database, prelude::*, transaction::TransactionMode};
use samod_core::StorageKey;
use wasm_bindgen::JsValue;

use crate::storage::LocalStorage;

/// A [`LocalStorage`] implementation for browser WASM which stores data in IndexedDB
#[derive(Clone)]
pub struct IndexedDbStorage {
    db_name: String,
    store_name: String,
}

impl IndexedDbStorage {
    pub fn new() -> Self {
        Self::with_names("samod_storage", "data")
    }

    pub fn with_names(db_name: &str, store_name: &str) -> Self {
        Self {
            db_name: db_name.to_string(),
            store_name: store_name.to_string(),
        }
    }

    async fn open_db(&self) -> Result<Database, JsValue> {
        let store_name = self.store_name.clone();
        Database::open(&self.db_name)
            .with_version(1u32)
            .with_on_upgrade_needed(move |_event, db| {
                // Create object store if it doesn't exist
                if !db.object_store_names().any(|name| name == store_name) {
                    let _ = db.create_object_store(&store_name).build();
                }
                Ok(())
            })
            .await
            .map_err(|_| JsValue::from_str("Failed to open IndexedDB connection"))
    }

    fn storage_key_to_js_value(key: &StorageKey) -> JsValue {
        JsValue::from_str(&key.to_string())
    }

    fn js_value_to_storage_key(value: JsValue) -> Result<StorageKey, JsValue> {
        let key_parts: Vec<String> = value
            .as_string()
            .ok_or_else(|| JsValue::from_str("Invalid key format"))?
            .split("/")
            .map(|s| s.to_string())
            .collect();
        StorageKey::from_parts(key_parts)
            .map_err(|_| JsValue::from_str("Failed to create StorageKey"))
    }
}

impl Default for IndexedDbStorage {
    fn default() -> Self {
        Self::new()
    }
}

impl LocalStorage for IndexedDbStorage {
    #[tracing::instrument(skip(self), level = "trace", ret)]
    fn load(&self, key: StorageKey) -> impl Future<Output = Option<Vec<u8>>> {
        let self_clone = self.clone();
        async move {
            match self_clone.open_db().await {
                Ok(db) => {
                    let tx = match db
                        .transaction(&self_clone.store_name)
                        .with_mode(TransactionMode::Readonly)
                        .build()
                    {
                        Ok(tx) => tx,
                        Err(e) => {
                            tracing::error!("Failed to create transaction: {:?}", e);
                            return None;
                        }
                    };

                    let store = match tx.object_store(&self_clone.store_name) {
                        Ok(store) => store,
                        Err(e) => {
                            tracing::error!("Failed to get object store: {:?}", e);
                            return None;
                        }
                    };

                    let js_key = Self::storage_key_to_js_value(&key);

                    match store.get(js_key).primitive() {
                        Ok(request) => match request.await {
                            Ok(Some(result)) => {
                                let uint8_array = js_sys::Uint8Array::new(&result);
                                Some(uint8_array.to_vec())
                            }
                            Ok(None) => None,
                            Err(_) => {
                                tracing::warn!("Failed to load from IndexedDB");
                                None
                            }
                        },
                        Err(e) => {
                            tracing::error!("Failed to create get request: {:?}", e);
                            None
                        }
                    }
                }
                Err(e) => {
                    tracing::error!("Failed to open database: {:?}", e);
                    None
                }
            }
        }
    }

    #[tracing::instrument(skip(self), level = "trace", ret)]
    fn load_range(
        &self,
        prefix: StorageKey,
    ) -> impl Future<Output = HashMap<StorageKey, Vec<u8>>> {
        let self_clone = self.clone();
        async move {
            let mut result = HashMap::new();

            match self_clone.open_db().await {
                Ok(db) => {
                    let tx = match db
                        .transaction(&self_clone.store_name)
                        .with_mode(TransactionMode::Readonly)
                        .build()
                    {
                        Ok(tx) => tx,
                        Err(e) => {
                            tracing::error!("Failed to create transaction: {:?}", e);
                            return HashMap::new();
                        }
                    };

                    let store = match tx.object_store(&self_clone.store_name) {
                        Ok(store) => store,
                        Err(e) => {
                            tracing::error!("Failed to get object store: {:?}", e);
                            return HashMap::new();
                        }
                    };

                    match store.open_cursor().await {
                        Ok(Some(mut cursor)) => loop {
                            match cursor.next_record::<JsValue>().await {
                                Ok(Some(js_value)) => {
                                    if let Ok(Some(js_key)) = cursor.key::<JsValue>() {
                                        if let Ok(storage_key) =
                                            Self::js_value_to_storage_key(js_key)
                                        {
                                            if prefix.is_prefix_of(&storage_key) {
                                                let uint8_array =
                                                    js_sys::Uint8Array::new(&js_value);
                                                result.insert(storage_key, uint8_array.to_vec());
                                            }
                                        }
                                    }
                                }
                                Ok(None) => break, // end of cursor
                                Err(_) => break,
                            }
                        },
                        Ok(None) => {}
                        Err(e) => {
                            tracing::error!("Failed to open cursor: {:?}", e);
                        }
                    }
                }
                Err(e) => {
                    tracing::error!("Failed to open database: {:?}", e);
                }
            }

            result
        }
    }

    #[tracing::instrument(skip(self, data), level = "trace")]
    fn put(&self, key: StorageKey, data: Vec<u8>) -> impl Future<Output = ()> {
        let self_clone = self.clone();
        async move {
            match self_clone.open_db().await {
                Ok(db) => {
                    let tx = match db
                        .transaction(&self_clone.store_name)
                        .with_mode(TransactionMode::Readwrite)
                        .build()
                    {
                        Ok(tx) => tx,
                        Err(e) => {
                            tracing::error!("Failed to create transaction: {:?}", e);
                            return;
                        }
                    };

                    let store = match tx.object_store(&self_clone.store_name) {
                        Ok(store) => store,
                        Err(e) => {
                            tracing::error!("Failed to get object store: {:?}", e);
                            return;
                        }
                    };

                    let js_key = Self::storage_key_to_js_value(&key);
                    let uint8_array = js_sys::Uint8Array::from(&data[..]);

                    let request = store.put(uint8_array.to_vec()).with_key(js_key);
                    match request.await {
                        Ok(_) => {
                            // Commit the transaction
                            if let Err(e) = tx.commit().await {
                                tracing::error!("Failed to commit transaction: {:?}", e);
                            }
                        }
                        Err(e) => {
                            tracing::error!("Failed to put data to IndexedDB: {:?}", e);
                        }
                    }
                }
                Err(e) => {
                    tracing::error!("Failed to open database: {:?}", e);
                }
            }
        }
    }

    #[tracing::instrument(skip(self), level = "trace")]
    fn delete(&self, key: StorageKey) -> impl Future<Output = ()> {
        let self_clone = self.clone();
        async move {
            match self_clone.open_db().await {
                Ok(db) => {
                    let tx = match db
                        .transaction(&self_clone.store_name)
                        .with_mode(TransactionMode::Readwrite)
                        .build()
                    {
                        Ok(tx) => tx,
                        Err(e) => {
                            tracing::error!("Failed to create transaction: {:?}", e);
                            return;
                        }
                    };

                    let store = match tx.object_store(&self_clone.store_name) {
                        Ok(store) => store,
                        Err(e) => {
                            tracing::error!("Failed to get object store: {:?}", e);
                            return;
                        }
                    };

                    let js_key = Self::storage_key_to_js_value(&key);

                    match store.delete(js_key).await {
                        Ok(_) => {
                            // Commit the transaction
                            if let Err(e) = tx.commit().await {
                                tracing::error!("Failed to commit transaction: {:?}", e);
                            }
                        }
                        Err(e) => {
                            tracing::error!("Failed to delete from IndexedDB: {:?}", e);
                        }
                    }
                }
                Err(e) => {
                    tracing::error!("Failed to open database: {:?}", e);
                }
            }
        }
    }
}