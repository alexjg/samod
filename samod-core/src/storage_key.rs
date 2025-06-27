use std::fmt;

use automerge::ChangeHash;

use crate::DocumentId;

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
/// let key1 = StorageKey::from(vec!["users", "123", "profile"]);
/// let key2 = StorageKey::from(vec!["users", "123", "settings"]);
/// let prefix = StorageKey::from(vec!["users", "123"]);
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

    pub fn incremental_path(doc_id: &DocumentId, change_hash: ChangeHash) -> StorageKey {
        StorageKey(vec![
            doc_id.to_string(),
            "incremental".to_string(),
            change_hash.to_string(),
        ])
    }

    pub fn snapshot_path(doc_id: DocumentId, compaction_hash: String) -> StorageKey {
        StorageKey(vec![
            doc_id.to_string(),
            "snapshot".to_string(),
            compaction_hash,
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
    /// let key = StorageKey::from_parts(&["users", "123", "profile"]);
    /// ```
    pub fn from_parts(parts: &[&str]) -> Self {
        StorageKey(parts.iter().map(|s| s.to_string()).collect())
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
    /// let key = StorageKey::from(vec!["a", "b", "c"]);
    /// let prefix = StorageKey::from(vec!["a", "b"]);
    /// assert_eq!(key.onelevel_deeper(&prefix), Some(StorageKey::from(vec!["a", "b", "c"])));
    ///
    /// let prefix2 = StorageKey::from(vec!["a"]);
    /// assert_eq!(key.onelevel_deeper(&prefix2), Some(StorageKey::from(vec!["a", "b"])));
    ///
    /// let prefix3 = StorageKey::from(vec!["a", "b", "c", "d"]);
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

    pub fn with_component(&self, component: String) -> StorageKey {
        let mut new_key = self.0.clone();
        new_key.push(component);
        StorageKey(new_key)
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

impl FromIterator<String> for StorageKey {
    fn from_iter<T: IntoIterator<Item = String>>(iter: T) -> Self {
        StorageKey(iter.into_iter().collect())
    }
}

impl<'a> FromIterator<&'a str> for StorageKey {
    fn from_iter<T: IntoIterator<Item = &'a str>>(iter: T) -> Self {
        StorageKey(iter.into_iter().map(String::from).collect())
    }
}

impl<'a> From<Vec<&'a str>> for StorageKey {
    fn from(vec: Vec<&'a str>) -> Self {
        StorageKey(vec.into_iter().map(String::from).collect())
    }
}

impl From<Vec<String>> for StorageKey {
    fn from(vec: Vec<String>) -> Self {
        StorageKey(vec)
    }
}

impl fmt::Display for StorageKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0.join("/"))
    }
}
