#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct StorageId(String);

impl From<uuid::Uuid> for StorageId {
    fn from(uuid: uuid::Uuid) -> Self {
        StorageId(uuid.to_string())
    }
}

impl From<String> for StorageId {
    fn from(s: String) -> Self {
        StorageId(s)
    }
}

impl From<&str> for StorageId {
    fn from(s: &str) -> Self {
        StorageId(s.to_string())
    }
}

impl std::fmt::Display for StorageId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl StorageId {
    pub fn new<R: rand::Rng>(rng: &mut R) -> Self {
        let bytes = rng.random();
        let uuid = uuid::Builder::from_random_bytes(bytes).into_uuid();
        StorageId(uuid.to_string())
    }

    pub(crate) fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StorageIdError {
    InvalidFormat,
}

impl std::fmt::Display for StorageIdError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            StorageIdError::InvalidFormat => write!(f, "Invalid storage ID format"),
        }
    }
}

impl std::error::Error for StorageIdError {}
