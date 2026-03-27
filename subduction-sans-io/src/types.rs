use std::fmt;

use sedimentree_core::crypto::digest::Digest;
use sedimentree_core::loose_commit::LooseCommit;

/// A peer identity, represented as an Ed25519 verifying key (32 bytes).
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct PeerId([u8; 32]);

impl PeerId {
    #[must_use]
    pub const fn new(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }

    #[must_use]
    pub const fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }

    #[must_use]
    pub fn from_verifying_key(key: &ed25519_dalek::VerifyingKey) -> Self {
        Self(key.to_bytes())
    }
}

impl fmt::Debug for PeerId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "PeerId({:02x}{:02x}..)", self.0[0], self.0[1])
    }
}

/// A timestamp in seconds since the Unix epoch.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct TimestampSeconds(u64);

impl TimestampSeconds {
    #[must_use]
    pub const fn new(secs: u64) -> Self {
        Self(secs)
    }

    #[must_use]
    pub const fn as_secs(&self) -> u64 {
        self.0
    }

    #[must_use]
    pub fn abs_diff(&self, other: TimestampSeconds) -> std::time::Duration {
        std::time::Duration::from_secs(self.0.abs_diff(other.0))
    }

    #[must_use]
    pub fn signed_diff(&self, other: TimestampSeconds) -> i64 {
        (self.0 as i64).wrapping_sub(other.0 as i64)
    }

    #[must_use]
    pub fn add_signed(&self, offset: i64) -> TimestampSeconds {
        TimestampSeconds((self.0 as i64).wrapping_add(offset) as u64)
    }
}

/// A discovery identifier for locating peers by service endpoint (32-byte hash).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct DiscoveryId([u8; 32]);

impl DiscoveryId {
    #[must_use]
    pub const fn from_raw(hash: [u8; 32]) -> Self {
        Self(hash)
    }

    #[must_use]
    pub const fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }
}

/// The intended recipient of a handshake challenge.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Audience {
    /// Known peer identity.
    Known(PeerId),
    /// Discovery mode: hash of URL or similar service identifier.
    Discover(DiscoveryId),
}

/// A peer's current heads for a sedimentree, with a monotonic counter for staleness filtering.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct RemoteHeads {
    /// Per-peer monotonic counter (incremented per send).
    pub counter: u64,
    /// The peer's current heads (tip commit digests).
    pub heads: Vec<Digest<LooseCommit>>,
}

/// A unique identifier for a batch sync request.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct RequestId {
    /// The peer that initiated the request.
    pub requestor: PeerId,
    /// A nonce unique to this request (per-peer).
    pub nonce: u64,
}
