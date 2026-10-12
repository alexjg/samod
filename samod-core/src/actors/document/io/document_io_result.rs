use crate::{ShareConfig, io::StorageResult};

#[derive(Debug, Clone)]
pub enum DocumentIoResult {
    Storage(StorageResult),
    CheckSharePolicy(ShareConfig),
}
