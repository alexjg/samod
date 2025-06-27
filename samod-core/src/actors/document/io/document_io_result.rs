use crate::io::StorageResult;

#[derive(Debug)]
pub enum DocumentIoResult {
    Storage(StorageResult),
    CheckAnnouncePolicy(bool),
}
