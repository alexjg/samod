use serde::{Deserialize, Serialize};
use std::str::FromStr;
use uuid::Uuid;

/// Unique identifier for an automerge document.
///
/// Document IDs are used throughout the sync protocol to identify which
/// document sync messages relate to. They are arbitrary byte arrays that
/// uniquely identify a document.
#[derive(Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct DocumentId(Uuid);

impl DocumentId {
    /// Creates a new random document ID.
    pub fn new<R: rand::Rng>(rng: &mut R) -> Self {
        let bytes: [u8; 16] = rng.random();
        let uuid = uuid::Builder::from_random_bytes(bytes).into_uuid();
        Self(uuid)
    }

    pub fn as_bytes(&self) -> &[u8] {
        self.0.as_bytes()
    }
}

impl std::fmt::Debug for DocumentId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let as_string = bs58::encode(&self.0).with_check().into_string();
        write!(f, "{as_string}")
    }
}

impl std::fmt::Display for DocumentId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let as_string = bs58::encode(&self.0).with_check().into_string();
        write!(f, "{as_string}")
    }
}

#[derive(Debug, thiserror::Error)]
#[error("Invalid document ID: {0}")]
pub struct BadDocumentId(String);

impl TryFrom<Vec<u8>> for DocumentId {
    type Error = BadDocumentId;

    fn try_from(v: Vec<u8>) -> Result<Self, Self::Error> {
        match uuid::Uuid::from_slice(v.as_slice()) {
            Ok(id) => Ok(Self(id)),
            Err(e) => Err(BadDocumentId(format!("invalid uuid: {e}"))),
        }
    }
}

impl FromStr for DocumentId {
    type Err = BadDocumentId;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match bs58::decode(s).with_check(None).into_vec() {
            Ok(bytes) => Self::try_from(bytes),
            Err(_) => {
                // attempt to parse legacy UUID format
                let uuid = uuid::Uuid::parse_str(s).map_err(|_| {
                    BadDocumentId(
                        "expected either a bs58-encoded document ID or a UUID".to_string(),
                    )
                })?;
                Ok(Self(uuid))
            }
        }
    }
}
