use serde::{Deserialize, Serialize};
use std::fmt;

/// Unique identifier for a peer in the sync network.
///
/// Peer IDs are ephemeral identifiers that identify a specific instance
/// of a peer (e.g., a browser tab, a process). They are UTF-8 strings
/// that are different from storage IDs which identify the underlying storage.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct PeerId(String);

impl PeerId {
    /// Creates a new peer ID with a generated value using the provided RNG.
    pub fn new_with_rng<R: rand::Rng>(rng: &mut R) -> Self {
        let id: u64 = rng.random();
        Self(format!("peer-{id}"))
    }

    /// Creates a peer ID from a string.
    pub fn from_string(s: String) -> Self {
        Self(s)
    }

    /// Returns the peer ID as a string slice.
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Returns the peer ID as a String.
    pub fn into_string(self) -> String {
        self.0
    }
}

impl fmt::Display for PeerId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl From<String> for PeerId {
    fn from(s: String) -> Self {
        PeerId(s)
    }
}

impl From<&str> for PeerId {
    fn from(s: &str) -> Self {
        PeerId(s.to_string())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PeerIdError {
    InvalidFormat,
}

impl fmt::Display for PeerIdError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PeerIdError::InvalidFormat => write!(f, "Invalid peer ID format"),
        }
    }
}

impl std::error::Error for PeerIdError {}
