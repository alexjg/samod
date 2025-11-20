use crate::io::StorageResult;

#[derive(Debug, Clone)]
pub enum DocumentIoResult {
    Storage(StorageResult),
    CheckAnnouncePolicy(bool),
}
