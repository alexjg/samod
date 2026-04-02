//! Storage key layout and KV operations for sedimentree data.
//!
//! This module defines the key structure used to store sedimentree data
//! in a hierarchical key-value store, and the abstract operations callers
//! must execute.
//!
//! # Key Layout
//!
//! ```text
//! sedimentree/{sed_id_hex}/commit/{digest_hex}    → Signed<LooseCommit> bytes
//! sedimentree/{sed_id_hex}/fragment/{digest_hex}  → Signed<Fragment> bytes
//! sedimentree/{sed_id_hex}/blob/{digest_hex}       → raw blob bytes
//! ```

use sedimentree_core::{
    blob::Blob,
    crypto::digest::Digest,
    fragment::Fragment,
    id::SedimentreeId,
    loose_commit::LooseCommit,
};

use std::collections::HashMap;

// ---------------------------------------------------------------------------
// StorageKey — a simple hierarchical path
// ---------------------------------------------------------------------------

/// A hierarchical key for storage operations.
///
/// Semantically identical to samod-core's `StorageKey` — a vector of path
/// components. Duplicated here to avoid a shared dependency.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct StorageKey(Vec<String>);

impl StorageKey {
    pub fn new(components: Vec<String>) -> Self {
        Self(components)
    }

    pub fn components(&self) -> &[String] {
        &self.0
    }

    pub fn is_prefix_of(&self, other: &StorageKey) -> bool {
        if self.0.len() > other.0.len() {
            return false;
        }
        self.0.iter().zip(other.0.iter()).all(|(a, b)| a == b)
    }
}

impl std::fmt::Display for StorageKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0.join("/"))
    }
}

// ---------------------------------------------------------------------------
// KV operations — what the caller must execute
// ---------------------------------------------------------------------------

/// A storage operation the caller must execute.
#[derive(Debug, Clone)]
pub enum KvOp {
    /// Load all key-value pairs matching a prefix.
    LoadRange { prefix: StorageKey },
    /// Load a single value by key.
    Load { key: StorageKey },
    /// Store a value at a key.
    Put { key: StorageKey, value: Vec<u8> },
}

/// Result of a storage operation.
#[derive(Debug, Clone)]
pub enum KvResult {
    LoadRange { values: HashMap<StorageKey, Vec<u8>> },
    Load { value: Option<Vec<u8>> },
    Put,
}

// ---------------------------------------------------------------------------
// Key layout functions
// ---------------------------------------------------------------------------

fn sed_id_hex(sed_id: &SedimentreeId) -> String {
    hex::encode(sed_id.as_bytes())
}

fn digest_hex<T>(digest: &Digest<T>) -> String {
    hex::encode(digest.as_bytes())
}

/// Prefix for all sedimentree data across all documents.
pub fn sedimentree_prefix() -> StorageKey {
    StorageKey::new(vec!["sedimentree".to_string()])
}

/// Prefix for all data belonging to a specific sedimentree.
pub fn sedimentree_id_prefix(sed_id: &SedimentreeId) -> StorageKey {
    StorageKey::new(vec!["sedimentree".to_string(), sed_id_hex(sed_id)])
}

/// Prefix for all commit metadata for a sedimentree.
pub fn sedimentree_commit_prefix(sed_id: &SedimentreeId) -> StorageKey {
    StorageKey::new(vec![
        "sedimentree".to_string(),
        sed_id_hex(sed_id),
        "commit".to_string(),
    ])
}

/// Path for a specific commit's signed metadata.
pub fn sedimentree_commit_path(
    sed_id: &SedimentreeId,
    digest: &Digest<LooseCommit>,
) -> StorageKey {
    StorageKey::new(vec![
        "sedimentree".to_string(),
        sed_id_hex(sed_id),
        "commit".to_string(),
        digest_hex(digest),
    ])
}

/// Prefix for all fragment metadata for a sedimentree.
pub fn sedimentree_fragment_prefix(sed_id: &SedimentreeId) -> StorageKey {
    StorageKey::new(vec![
        "sedimentree".to_string(),
        sed_id_hex(sed_id),
        "fragment".to_string(),
    ])
}

/// Path for a specific fragment's signed metadata.
pub fn sedimentree_fragment_path(
    sed_id: &SedimentreeId,
    digest: &Digest<Fragment>,
) -> StorageKey {
    StorageKey::new(vec![
        "sedimentree".to_string(),
        sed_id_hex(sed_id),
        "fragment".to_string(),
        digest_hex(digest),
    ])
}

/// Path for a specific blob (content-addressed by blob digest).
pub fn sedimentree_blob_path(
    sed_id: &SedimentreeId,
    digest: &Digest<Blob>,
) -> StorageKey {
    StorageKey::new(vec![
        "sedimentree".to_string(),
        sed_id_hex(sed_id),
        "blob".to_string(),
        digest_hex(digest),
    ])
}

// ---------------------------------------------------------------------------
// Serialization helpers
// ---------------------------------------------------------------------------

use sedimentree_core::sedimentree::Sedimentree;
use subduction_crypto::signed::Signed;

use crate::batch_sync::BatchSyncIoResult;

/// Compute the digest of a `LooseCommit` inside a `Signed<LooseCommit>`.
pub fn compute_commit_digest(signed: &Signed<LooseCommit>) -> Digest<LooseCommit> {
    if let Ok(payload) = signed.try_decode_trusted_payload() {
        Digest::hash(&payload)
    } else {
        // Fallback: hash the raw signed bytes (not ideal)
        Digest::force_from_bytes(*blake3::hash(signed.as_bytes()).as_bytes())
    }
}

/// Compute the digest of a `Fragment` inside a `Signed<Fragment>`.
pub fn compute_fragment_digest(signed: &Signed<Fragment>) -> Digest<Fragment> {
    if let Ok(payload) = signed.try_decode_trusted_payload() {
        Digest::hash(&payload)
    } else {
        Digest::force_from_bytes(*blake3::hash(signed.as_bytes()).as_bytes())
    }
}

/// Produce KV put operations for storing commits and their blobs.
pub fn commits_to_kv_ops(
    sed_id: &SedimentreeId,
    commits: &[(Signed<LooseCommit>, Blob)],
) -> Vec<KvOp> {
    let mut ops = Vec::with_capacity(commits.len() * 2);
    for (signed, blob) in commits {
        let commit_digest = compute_commit_digest(signed);
        let blob_digest = blob.meta().digest();
        ops.push(KvOp::Put {
            key: sedimentree_commit_path(sed_id, &commit_digest),
            value: signed.as_bytes().to_vec(),
        });
        ops.push(KvOp::Put {
            key: sedimentree_blob_path(sed_id, &blob_digest),
            value: blob.as_slice().to_vec(),
        });
    }
    ops
}

/// Produce KV put operations for storing fragments and their blobs.
pub fn fragments_to_kv_ops(
    sed_id: &SedimentreeId,
    fragments: &[(Signed<Fragment>, Blob)],
) -> Vec<KvOp> {
    let mut ops = Vec::with_capacity(fragments.len() * 2);
    for (signed, blob) in fragments {
        let frag_digest = compute_fragment_digest(signed);
        let blob_digest = blob.meta().digest();
        ops.push(KvOp::Put {
            key: sedimentree_fragment_path(sed_id, &frag_digest),
            value: signed.as_bytes().to_vec(),
        });
        ops.push(KvOp::Put {
            key: sedimentree_blob_path(sed_id, &blob_digest),
            value: blob.as_slice().to_vec(),
        });
    }
    ops
}

/// Reassemble a sedimentree from a LoadRange result containing all data
/// under the sedimentree prefix.
pub fn reassemble_sedimentree_from_range(
    sed_id: SedimentreeId,
    data: HashMap<StorageKey, Vec<u8>>,
) -> BatchSyncIoResult {
    let sid = sed_id_hex(&sed_id);
    let commit_prefix = format!("sedimentree/{}/commit/", sid);
    let fragment_prefix = format!("sedimentree/{}/fragment/", sid);
    let blob_prefix = format!("sedimentree/{}/blob/", sid);

    let mut commit_bytes_map: HashMap<String, Vec<u8>> = HashMap::new();
    let mut fragment_bytes_map: HashMap<String, Vec<u8>> = HashMap::new();
    let mut blob_bytes_map: HashMap<String, Vec<u8>> = HashMap::new();

    for (key, value) in &data {
        let key_str = key.to_string();
        if let Some(suffix) = key_str.strip_prefix(&commit_prefix) {
            commit_bytes_map.insert(suffix.to_string(), value.clone());
        } else if let Some(suffix) = key_str.strip_prefix(&fragment_prefix) {
            fragment_bytes_map.insert(suffix.to_string(), value.clone());
        } else if let Some(suffix) = key_str.strip_prefix(&blob_prefix) {
            blob_bytes_map.insert(suffix.to_string(), value.clone());
        }
    }

    let mut commits = HashMap::new();
    let mut loose_commits = Vec::new();
    for (_digest_hex, bytes) in &commit_bytes_map {
        if let Ok(signed) = Signed::<LooseCommit>::try_decode(bytes.clone()) {
            if let Ok(verified) = signed.try_verify() {
                let commit = verified.payload().clone();
                let digest = Digest::hash(&commit);
                let blob_digest_hex = hex::encode(commit.blob_meta().digest().as_bytes());
                let blob = blob_bytes_map
                    .get(&blob_digest_hex)
                    .map(|b| Blob::new(b.clone()))
                    .unwrap_or_else(|| Blob::new(vec![]));
                loose_commits.push(commit);
                commits.insert(digest, (signed, blob));
            }
        }
    }

    let mut fragments_map = HashMap::new();
    let mut fragment_list = Vec::new();
    for (_digest_hex, bytes) in &fragment_bytes_map {
        if let Ok(signed) = Signed::<Fragment>::try_decode(bytes.clone()) {
            if let Ok(verified) = signed.try_verify() {
                let frag = verified.payload().clone();
                let digest = Digest::hash(&frag);
                let blob_digest_hex = hex::encode(frag.summary().blob_meta().digest().as_bytes());
                let blob = blob_bytes_map
                    .get(&blob_digest_hex)
                    .map(|b| Blob::new(b.clone()))
                    .unwrap_or_else(|| Blob::new(vec![]));
                fragment_list.push(frag);
                fragments_map.insert(digest, (signed, blob));
            }
        }
    }

    let raw = Sedimentree::new(fragment_list, loose_commits);
    let minimized = raw.minimize(&sedimentree_core::commit::CountLeadingZeroBytes);

    // Filter the signed data maps to only include items in the minimized tree.
    let minimized_commit_digests: std::collections::HashSet<_> =
        minimized.commit_entries().map(|(d, _)| *d).collect();
    let minimized_fragment_digests: std::collections::HashSet<_> =
        minimized.fragment_entries().map(|(d, _)| *d).collect();
    commits.retain(|d, _| minimized_commit_digests.contains(d));
    fragments_map.retain(|d, _| minimized_fragment_digests.contains(d));

    BatchSyncIoResult::Sedimentree {
        id: sed_id,
        sedimentree: minimized,
        commits,
        fragments: fragments_map,
    }
}

/// Pair signed commit bytes with loaded blob bytes.
pub fn assemble_commit_pairs(
    signed_commits: HashMap<Digest<LooseCommit>, Vec<u8>>,
    loaded_blobs: HashMap<Digest<LooseCommit>, Vec<u8>>,
) -> Vec<(Signed<LooseCommit>, Blob)> {
    let mut result = Vec::new();
    for (digest, commit_bytes) in signed_commits {
        if let Ok(signed) = Signed::<LooseCommit>::try_decode(commit_bytes) {
            let blob = loaded_blobs
                .get(&digest)
                .map(|b| Blob::new(b.clone()))
                .unwrap_or_else(|| Blob::new(vec![]));
            result.push((signed, blob));
        }
    }
    result
}

/// Pair signed fragment bytes with loaded blob bytes.
pub fn assemble_fragment_pairs(
    signed_fragments: HashMap<Digest<Fragment>, Vec<u8>>,
    loaded_blobs: HashMap<Digest<Fragment>, Vec<u8>>,
) -> Vec<(Signed<Fragment>, Blob)> {
    let mut result = Vec::new();
    for (digest, frag_bytes) in signed_fragments {
        if let Ok(signed) = Signed::<Fragment>::try_decode(frag_bytes) {
            let blob = loaded_blobs
                .get(&digest)
                .map(|b| Blob::new(b.clone()))
                .unwrap_or_else(|| Blob::new(vec![]));
            result.push((signed, blob));
        }
    }
    result
}
