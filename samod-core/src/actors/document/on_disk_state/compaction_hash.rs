use automerge::ChangeHash;
use sha2::Digest;

/// The SHA-256 hash of a set of change hashes, used to uniquely identify a compacted document
#[derive(Copy, Clone, PartialEq, Eq, Hash, Ord, PartialOrd)]
pub struct CompactionHash([u8; 32]);

impl CompactionHash {
    pub fn new(change_hashes: &[ChangeHash]) -> Self {
        let mut hasher = sha2::Sha256::new();
        for hash in change_hashes {
            hasher.update(hash.as_ref());
        }
        let hash_result = hasher.finalize();
        Self(hash_result.into())
    }
}

impl std::fmt::Debug for CompactionHash {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "CompactionHash({})", hex::encode(self.0))
    }
}

impl std::fmt::Display for CompactionHash {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", hex::encode(self.0))
    }
}
