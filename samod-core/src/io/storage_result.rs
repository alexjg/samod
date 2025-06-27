use std::collections::HashMap;

use crate::StorageKey;

// The inverse of `IoAction`
#[derive(Debug)]
pub enum StorageResult {
    Load {
        value: Option<Vec<u8>>,
    },
    LoadRange {
        values: HashMap<StorageKey, Vec<u8>>,
    },
    Put,
    Delete,
}
