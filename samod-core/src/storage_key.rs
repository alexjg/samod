use std::fmt;

use automerge::ChangeHash;

use crate::{CompactionHash, DocumentId};

/// A hierarchical key for storage operations in the samod-core system.
///
/// `StorageKey` represents a path-like key structure that supports efficient
/// prefix-based operations. Keys are composed of string components that form
/// a hierarchy, similar to filesystem paths or namespaces.
///
/// ## Usage
///
/// Storage keys are used throughout samod-core for organizing data in the
/// key-value store. They support operations like prefix matching for range
/// queries and hierarchical organization of related data.
///
/// ## Examples
///
/// ```rust
/// use samod_core::StorageKey;
///
/// // Create keys from string vectors
/// let key1 = StorageKey::from_parts(vec!["users", "123", "profile"]).unwrap();
/// let key2 = StorageKey::from_parts(vec!["users", "123", "settings"]).unwrap();
/// let prefix = StorageKey::from_parts(vec!["users", "123"]).unwrap();
///
/// // Check prefix relationships
/// assert!(prefix.is_prefix_of(&key1));
/// assert!(prefix.is_prefix_of(&key2));
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct StorageKey(Vec<String>);

impl StorageKey {
    pub fn storage_id_path() -> StorageKey {
        StorageKey(vec!["storage-adapter-id".to_string()])
    }

    pub fn incremental_prefix(doc_id: &DocumentId) -> StorageKey {
        StorageKey(vec![doc_id.to_string(), "incremental".to_string()])
    }

    pub fn incremental_path(doc_id: &DocumentId, change_hash: ChangeHash) -> StorageKey {
        StorageKey(vec![
            doc_id.to_string(),
            "incremental".to_string(),
            change_hash.to_string(),
        ])
    }

    pub fn snapshot_prefix(doc_id: &DocumentId) -> StorageKey {
        StorageKey(vec![doc_id.to_string(), "snapshot".to_string()])
    }

    pub fn snapshot_path(doc_id: &DocumentId, compaction_hash: &CompactionHash) -> StorageKey {
        StorageKey(vec![
            doc_id.to_string(),
            "snapshot".to_string(),
            compaction_hash.to_string(),
        ])
    }

    /// Creates a storage key from a slice of string parts.
    ///
    /// # Arguments
    ///
    /// * `parts` - The parts that make up the key path
    ///
    /// # Example
    ///
    /// ```rust
    /// use samod_core::StorageKey;
    ///
    /// let key = StorageKey::from_parts(&["users", "123", "profile"]).unwrap();
    /// ```
    ///
    /// # Errors
    ///
    /// Returns an error if any part is empty or contains a slash.
    pub fn from_parts<I: IntoIterator<Item = S>, S: AsRef<str>>(
        parts: I,
    ) -> Result<Self, InvalidStorageKey> {
        let mut components = Vec::new();
        for part in parts {
            if part.as_ref().is_empty() || part.as_ref().contains("/") {
                return Err(InvalidStorageKey);
            }
            components.push(part.as_ref().to_string());
        }
        Ok(StorageKey(components))
    }

    /// Checks if this key is a prefix of another key.
    ///
    /// # Arguments
    ///
    /// * `other` - The key to check against
    pub fn is_prefix_of(&self, other: &StorageKey) -> bool {
        if self.0.len() > other.0.len() {
            return false;
        }
        self.0.iter().zip(other.0.iter()).all(|(a, b)| a == b)
    }

    /// Checks if this key is one level deeper then  the given prefix
    ///
    /// # Examples
    ///
    /// ```rust
    /// # use samod_core::StorageKey;
    /// let key = StorageKey::from_parts(vec!["a", "b", "c"]).unwrap();
    /// let prefix = StorageKey::from_parts(vec!["a", "b"]).unwrap();
    /// assert_eq!(key.onelevel_deeper(&prefix), Some(StorageKey::from_parts(vec!["a", "b", "c"]).unwrap()));
    ///
    /// let prefix2 = StorageKey::from_parts(vec!["a"]).unwrap();
    /// assert_eq!(key.onelevel_deeper(&prefix2), Some(StorageKey::from_parts(vec!["a", "b"]).unwrap()));
    ///
    /// let prefix3 = StorageKey::from_parts(vec!["a", "b", "c", "d"]).unwrap();
    /// assert_eq!(key.onelevel_deeper(&prefix3), None);
    /// ```
    pub fn onelevel_deeper(&self, prefix: &StorageKey) -> Option<StorageKey> {
        if prefix.is_prefix_of(self) && self.0.len() > prefix.0.len() {
            let components = self.0.iter().take(prefix.0.len() + 1).cloned();
            Some(StorageKey(components.collect()))
        } else {
            None
        }
    }

    pub fn with_suffix(&self, suffix: StorageKey) -> StorageKey {
        let mut new_key = self.0.clone();
        new_key.extend(suffix.0);
        StorageKey(new_key)
    }

    /// Create a new StorageKey with the given component appended.
    ///
    /// # Errors
    ///
    /// Returns an error if the new component is empty or contains a forward slash.
    pub fn with_component(&self, component: String) -> Result<StorageKey, InvalidStorageKey> {
        if component.is_empty() || component.contains('/') {
            Err(InvalidStorageKey)
        } else {
            let mut new_key = self.0.clone();
            new_key.push(component);
            Ok(StorageKey(new_key))
        }
    }
}

impl IntoIterator for StorageKey {
    type Item = String;
    type IntoIter = std::vec::IntoIter<String>;

    fn into_iter(self) -> Self::IntoIter {
        self.0.into_iter()
    }
}

impl<'a> IntoIterator for &'a StorageKey {
    type Item = &'a String;
    type IntoIter = std::slice::Iter<'a, String>;

    fn into_iter(self) -> Self::IntoIter {
        self.0.iter()
    }
}

impl fmt::Display for StorageKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0.join("/"))
    }
}

#[derive(Debug)]
pub struct InvalidStorageKey;

impl std::fmt::Display for InvalidStorageKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "InvalidStorageKey")
    }
}

impl std::error::Error for InvalidStorageKey {}
