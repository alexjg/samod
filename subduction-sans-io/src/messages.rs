//! Sync message codec for the subduction protocol.
//!
//! # Wire Layout
//!
//! ```text
//! +--------+-----------+-----+-----------------------------+
//! | Schema | TotalSize | Tag |         Payload             |
//! |   4B   |    4B     | 1B  |         variable            |
//! +--------+-----------+-----+-----------------------------+
//! ```
//!
//! - **Schema**: `SUM\x00` (4 bytes)
//! - **TotalSize**: Total message size in bytes (big-endian u32)
//! - **Tag**: Variant discriminant (u8)
//! - **Payload**: Variant-specific data

use std::collections::BTreeSet;

use sedimentree_core::{
    blob::Blob,
    codec::error::{
        Bijou64Error, BlobTooLarge, BufferTooShort, DecodeError, InvalidEnumTag, InvalidSchema,
        ReadingType, SizeMismatch,
    },
    crypto::{
        digest::Digest,
        fingerprint::{Fingerprint, FingerprintSeed},
    },
    fragment::{Fragment, id::FragmentId},
    id::SedimentreeId,
    loose_commit::{LooseCommit, id::CommitId},
    sedimentree::FingerprintSummary,
};
use subduction_crypto::signed::Signed;

use crate::types::{PeerId, RemoteHeads, RequestId};

// ============================================================================
// Types
// ============================================================================

/// The sync messages exchanged over a connection.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SyncMessage {
    /// A single loose commit being sent for a particular sedimentree.
    LooseCommit {
        /// The ID of the sedimentree that this commit belongs to.
        id: SedimentreeId,
        /// The signed loose commit being sent.
        commit: Signed<LooseCommit>,
        /// The blob containing the commit data.
        blob: Blob,
        /// The sender's heads for this sedimentree after ingesting this commit.
        sender_heads: RemoteHeads,
    },

    /// A single fragment being sent for a particular sedimentree.
    Fragment {
        /// The ID of the sedimentree that this fragment belongs to.
        id: SedimentreeId,
        /// The signed fragment being sent.
        fragment: Signed<Fragment>,
        /// The blob containing the fragment data.
        blob: Blob,
        /// The sender's heads for this sedimentree after ingesting this fragment.
        sender_heads: RemoteHeads,
    },

    /// A request for blobs by their digests within a specific sedimentree.
    BlobsRequest {
        /// The sedimentree to fetch blobs from.
        id: SedimentreeId,
        /// The blob digests being requested.
        digests: Vec<Digest<Blob>>,
    },

    /// A response to a blobs request with blobs for a specific sedimentree.
    BlobsResponse {
        /// The sedimentree these blobs belong to.
        id: SedimentreeId,
        /// The requested blobs.
        blobs: Vec<Blob>,
    },

    /// A request to batch sync an entire sedimentree.
    BatchSyncRequest(BatchSyncRequest),

    /// A response to a batch sync request.
    BatchSyncResponse(BatchSyncResponse),

    /// A request to remove subscriptions from specific sedimentrees.
    RemoveSubscriptions(RemoveSubscriptions),

    /// Notification that a data request was rejected due to authorization failure.
    DataRequestRejected(DataRequestRejected),

    /// Notification of a peer's current heads for a sedimentree.
    HeadsUpdate {
        /// The sedimentree these heads are for.
        id: SedimentreeId,
        /// The sender's current heads after ingesting data.
        heads: RemoteHeads,
    },
}

/// A request to sync a sedimentree in batch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BatchSyncRequest {
    /// The ID of the sedimentree to sync.
    pub id: SedimentreeId,
    /// The unique ID of the request.
    pub req_id: RequestId,
    /// Compact fingerprint summary of the requester's sedimentree.
    pub fingerprint_summary: FingerprintSummary,
    /// Whether to subscribe to future updates for this sedimentree.
    pub subscribe: bool,
}

/// A response to a batch sync request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BatchSyncResponse {
    /// The ID of the request that this is a response to.
    pub req_id: RequestId,
    /// The ID of the sedimentree that was synced.
    pub id: SedimentreeId,
    /// The result of the sync request.
    pub result: SyncResult,
    /// The responder's heads for this sedimentree at the time the response was generated.
    pub responder_heads: RemoteHeads,
}

/// The result of a batch sync request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SyncResult {
    /// Sync succeeded.
    Ok(SyncDiff),
    /// Sedimentree not found.
    NotFound,
    /// Peer is not authorized to access this sedimentree.
    Unauthorized,
}

/// The calculated difference between two peers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SyncDiff {
    /// Commits the requestor is missing (responder sends these).
    pub missing_commits: Vec<(Signed<LooseCommit>, Blob)>,
    /// Fragments the requestor is missing (responder sends these).
    pub missing_fragments: Vec<(Signed<Fragment>, Blob)>,
    /// Data the responder is requesting from the requestor.
    pub requesting: RequestedData,
}

/// Data that the responder is requesting from the requestor.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct RequestedData {
    /// Fingerprints of commits the responder needs from the requestor.
    pub commit_fingerprints: Vec<Fingerprint<CommitId>>,
    /// Fingerprints of fragments the responder needs from the requestor.
    pub fragment_fingerprints: Vec<Fingerprint<FragmentId>>,
}

impl RequestedData {
    /// Returns `true` if there is no data being requested.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.commit_fingerprints.is_empty() && self.fragment_fingerprints.is_empty()
    }
}

/// A request to remove subscriptions from specific sedimentrees.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemoveSubscriptions {
    /// The IDs of the sedimentrees to unsubscribe from.
    pub ids: Vec<SedimentreeId>,
}

/// Sent when a peer's data request cannot be fulfilled due to authorization failure.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DataRequestRejected {
    /// The sedimentree ID that was rejected.
    pub id: SedimentreeId,
}

// ============================================================================
// Convenience conversions
// ============================================================================

impl From<BatchSyncRequest> for SyncMessage {
    fn from(req: BatchSyncRequest) -> Self {
        SyncMessage::BatchSyncRequest(req)
    }
}

impl From<BatchSyncResponse> for SyncMessage {
    fn from(resp: BatchSyncResponse) -> Self {
        SyncMessage::BatchSyncResponse(resp)
    }
}

impl From<RemoveSubscriptions> for SyncMessage {
    fn from(unsub: RemoveSubscriptions) -> Self {
        SyncMessage::RemoveSubscriptions(unsub)
    }
}

impl From<DataRequestRejected> for SyncMessage {
    fn from(rejection: DataRequestRejected) -> Self {
        SyncMessage::DataRequestRejected(rejection)
    }
}

// ============================================================================
// Binary Codec
// ============================================================================

/// Schema header for the `SyncMessage` envelope.
pub const MESSAGE_SCHEMA: [u8; 4] = *b"SUM\x00";

/// Minimum size of a message envelope (schema + total_size + tag).
const ENVELOPE_HEADER_SIZE: usize = 4 + 4 + 1; // 9 bytes

mod tags {
    pub(super) const LOOSE_COMMIT: u8 = 0x00;
    pub(super) const FRAGMENT: u8 = 0x01;
    pub(super) const BLOBS_REQUEST: u8 = 0x02;
    pub(super) const BLOBS_RESPONSE: u8 = 0x03;
    pub(super) const BATCH_SYNC_REQUEST: u8 = 0x04;
    pub(super) const BATCH_SYNC_RESPONSE: u8 = 0x05;
    pub(super) const REMOVE_SUBSCRIPTIONS: u8 = 0x06;
    pub(super) const DATA_REQUEST_REJECTED: u8 = 0x07;
    pub(super) const HEADS_UPDATE: u8 = 0x08;
}

mod min_sizes {
    // sed_id(32) + counter(8) + heads_count(4) + Signed<LooseCommit>::MIN_SIZE(166) + blob_len_prefix(bijou64 min=1)
    pub(super) const LOOSE_COMMIT: usize = 32 + 8 + 4 + 166 + 1;
    pub(super) const FRAGMENT: usize = 32 + 8 + 4 + 200 + 1;
    pub(super) const BLOBS_REQUEST: usize = 32 + 2;
    pub(super) const BLOBS_RESPONSE: usize = 32 + 2;
    pub(super) const BATCH_SYNC_REQUEST: usize = 32 + 32 + 8 + 1 + 16 + 2 + 2;
    // requestor(32) + nonce(8) + sed_id(32) + result_tag(1) + counter(8) + heads_count(4)
    pub(super) const BATCH_SYNC_RESPONSE: usize = 32 + 8 + 32 + 1 + 8 + 4;
    pub(super) const REMOVE_SUBSCRIPTIONS: usize = 2;
    pub(super) const DATA_REQUEST_REJECTED: usize = 32;
    // sed_id(32) + counter(8) + head_count(4)
    pub(super) const HEADS_UPDATE: usize = 32 + 8 + 4;
}

mod result_tags {
    pub(super) const OK: u8 = 0x00;
    pub(super) const NOT_FOUND: u8 = 0x01;
    pub(super) const UNAUTHORIZED: u8 = 0x02;
}

impl SyncMessage {
    /// Encode the message to wire bytes.
    #[must_use]
    pub fn encode(&self) -> Vec<u8> {
        encode_message(self)
    }

    /// Decode a message from wire bytes.
    ///
    /// # Errors
    ///
    /// Returns an error if the message is malformed.
    pub fn try_decode(bytes: &[u8]) -> Result<Self, DecodeError> {
        decode_message(bytes)
    }

    /// Get the request ID for this message, if any.
    #[must_use]
    pub const fn request_id(&self) -> Option<RequestId> {
        match self {
            SyncMessage::BatchSyncRequest(BatchSyncRequest { req_id, .. })
            | SyncMessage::BatchSyncResponse(BatchSyncResponse { req_id, .. }) => Some(*req_id),
            SyncMessage::LooseCommit { .. }
            | SyncMessage::Fragment { .. }
            | SyncMessage::BlobsRequest { .. }
            | SyncMessage::BlobsResponse { .. }
            | SyncMessage::RemoveSubscriptions(_)
            | SyncMessage::DataRequestRejected(_)
            | SyncMessage::HeadsUpdate { .. } => None,
        }
    }

    /// Get the variant name of this message for logging purposes.
    #[must_use]
    pub const fn variant_name(&self) -> &'static str {
        match self {
            SyncMessage::LooseCommit { .. } => "LooseCommit",
            SyncMessage::Fragment { .. } => "Fragment",
            SyncMessage::BlobsRequest { .. } => "BlobsRequest",
            SyncMessage::BlobsResponse { .. } => "BlobsResponse",
            SyncMessage::BatchSyncRequest(_) => "BatchSyncRequest",
            SyncMessage::BatchSyncResponse(_) => "BatchSyncResponse",
            SyncMessage::RemoveSubscriptions(_) => "RemoveSubscriptions",
            SyncMessage::DataRequestRejected(_) => "DataRequestRejected",
            SyncMessage::HeadsUpdate { .. } => "HeadsUpdate",
        }
    }

    /// Get the sedimentree ID associated with this message, if any.
    #[must_use]
    pub const fn sedimentree_id(&self) -> Option<SedimentreeId> {
        match self {
            SyncMessage::LooseCommit { id, .. }
            | SyncMessage::Fragment { id, .. }
            | SyncMessage::BlobsRequest { id, .. }
            | SyncMessage::BlobsResponse { id, .. }
            | SyncMessage::BatchSyncRequest(BatchSyncRequest { id, .. })
            | SyncMessage::BatchSyncResponse(BatchSyncResponse { id, .. })
            | SyncMessage::DataRequestRejected(DataRequestRejected { id })
            | SyncMessage::HeadsUpdate { id, .. } => Some(*id),
            SyncMessage::RemoveSubscriptions(_) => None,
        }
    }

    fn payload_size(&self) -> usize {
        match self {
            SyncMessage::LooseCommit {
                commit,
                blob,
                sender_heads,
                ..
            } => {
                32 + commit.as_bytes().len()
                    + bijou64::encoded_len(blob.as_slice().len() as u64)
                    + blob.as_slice().len()
                    + remote_heads_size(sender_heads)
            }
            SyncMessage::Fragment {
                fragment,
                blob,
                sender_heads,
                ..
            } => {
                32 + fragment.as_bytes().len()
                    + bijou64::encoded_len(blob.as_slice().len() as u64)
                    + blob.as_slice().len()
                    + remote_heads_size(sender_heads)
            }
            SyncMessage::BlobsRequest { digests, .. } => 32 + 2 + (digests.len() * 32),
            SyncMessage::BlobsResponse { blobs, .. } => {
                32 + 2
                    + blobs
                        .iter()
                        .map(|b| {
                            bijou64::encoded_len(b.as_slice().len() as u64) + b.as_slice().len()
                        })
                        .sum::<usize>()
            }
            SyncMessage::BatchSyncRequest(req) => {
                32 + 32
                    + 8
                    + 1
                    + 16
                    + 2
                    + 2
                    + (req.fingerprint_summary.commit_fingerprints().len() * 8)
                    + (req.fingerprint_summary.fragment_fingerprints().len() * 8)
            }
            SyncMessage::BatchSyncResponse(resp) => {
                32 + 8
                    + 32
                    + 1
                    + sync_result_size(&resp.result)
                    + remote_heads_size(&resp.responder_heads)
            }
            SyncMessage::RemoveSubscriptions(unsub) => 2 + (unsub.ids.len() * 32),
            SyncMessage::DataRequestRejected(_) => 32,
            SyncMessage::HeadsUpdate { heads, .. } => 32 + remote_heads_size(heads),
        }
    }
}

/// Size of a `RemoteHeads` on the wire: u64 counter + u32 count + 32 bytes per digest.
const fn remote_heads_size(heads: &RemoteHeads) -> usize {
    8 + 4 + heads.heads.len() * 32
}

fn sync_result_size(result: &SyncResult) -> usize {
    match result {
        SyncResult::Ok(diff) => sync_diff_size(diff),
        SyncResult::NotFound | SyncResult::Unauthorized => 0,
    }
}

fn sync_diff_size(diff: &SyncDiff) -> usize {
    let counts_size = 2 + 2 + 2 + 2;

    let commits_size: usize = diff
        .missing_commits
        .iter()
        .map(|(signed, blob)| {
            signed.as_bytes().len()
                + bijou64::encoded_len(blob.as_slice().len() as u64)
                + blob.as_slice().len()
        })
        .sum();

    let fragments_size: usize = diff
        .missing_fragments
        .iter()
        .map(|(signed, blob)| {
            signed.as_bytes().len()
                + bijou64::encoded_len(blob.as_slice().len() as u64)
                + blob.as_slice().len()
        })
        .sum();

    let requested_fps_size = (diff.requesting.commit_fingerprints.len()
        + diff.requesting.fragment_fingerprints.len())
        * 8;

    counts_size + commits_size + fragments_size + requested_fps_size
}

// ============================================================================
// Encode
// ============================================================================

fn encode_message(msg: &SyncMessage) -> Vec<u8> {
    let payload_size = msg.payload_size();
    let total_size = ENVELOPE_HEADER_SIZE + payload_size;

    let mut buf = Vec::with_capacity(total_size);

    buf.extend_from_slice(&MESSAGE_SCHEMA);

    #[allow(clippy::cast_possible_truncation)]
    buf.extend_from_slice(&(total_size as u32).to_be_bytes());

    match msg {
        SyncMessage::LooseCommit {
            id,
            commit,
            blob,
            sender_heads,
        } => {
            buf.push(tags::LOOSE_COMMIT);
            buf.extend_from_slice(id.as_bytes());
            encode_remote_heads(&mut buf, sender_heads);
            buf.extend_from_slice(commit.as_bytes());
            bijou64::encode(blob.as_slice().len() as u64, &mut buf);
            buf.extend_from_slice(blob.as_slice());
        }
        SyncMessage::Fragment {
            id,
            fragment,
            blob,
            sender_heads,
        } => {
            buf.push(tags::FRAGMENT);
            buf.extend_from_slice(id.as_bytes());
            encode_remote_heads(&mut buf, sender_heads);
            buf.extend_from_slice(fragment.as_bytes());
            bijou64::encode(blob.as_slice().len() as u64, &mut buf);
            buf.extend_from_slice(blob.as_slice());
        }
        SyncMessage::BlobsRequest { id, digests } => {
            buf.push(tags::BLOBS_REQUEST);
            encode_blobs_request(&mut buf, id, digests);
        }
        SyncMessage::BlobsResponse { id, blobs } => {
            buf.push(tags::BLOBS_RESPONSE);
            encode_blobs_response(&mut buf, id, blobs);
        }
        SyncMessage::BatchSyncRequest(req) => {
            buf.push(tags::BATCH_SYNC_REQUEST);
            encode_batch_sync_request(&mut buf, req);
        }
        SyncMessage::BatchSyncResponse(resp) => {
            buf.push(tags::BATCH_SYNC_RESPONSE);
            encode_batch_sync_response(&mut buf, resp);
        }
        SyncMessage::RemoveSubscriptions(unsub) => {
            buf.push(tags::REMOVE_SUBSCRIPTIONS);
            encode_remove_subscriptions(&mut buf, unsub);
        }
        SyncMessage::DataRequestRejected(rejected) => {
            buf.push(tags::DATA_REQUEST_REJECTED);
            encode_data_request_rejected(&mut buf, rejected);
        }
        SyncMessage::HeadsUpdate { id, heads } => {
            buf.push(tags::HEADS_UPDATE);
            buf.extend_from_slice(id.as_bytes());
            encode_remote_heads(&mut buf, heads);
        }
    }

    buf
}

fn encode_remote_heads(buf: &mut Vec<u8>, remote_heads: &RemoteHeads) {
    buf.extend_from_slice(&remote_heads.counter.to_be_bytes());
    #[allow(clippy::cast_possible_truncation)]
    buf.extend_from_slice(&(remote_heads.heads.len() as u32).to_be_bytes());
    for digest in &remote_heads.heads {
        buf.extend_from_slice(digest.as_bytes());
    }
}

fn encode_blobs_request(buf: &mut Vec<u8>, id: &SedimentreeId, digests: &[Digest<Blob>]) {
    buf.extend_from_slice(id.as_bytes());
    #[allow(clippy::cast_possible_truncation)]
    buf.extend_from_slice(&(digests.len() as u16).to_be_bytes());
    for digest in digests {
        buf.extend_from_slice(digest.as_bytes());
    }
}

fn encode_blobs_response(buf: &mut Vec<u8>, id: &SedimentreeId, blobs: &[Blob]) {
    buf.extend_from_slice(id.as_bytes());
    #[allow(clippy::cast_possible_truncation)]
    buf.extend_from_slice(&(blobs.len() as u16).to_be_bytes());
    for blob in blobs {
        bijou64::encode(blob.as_slice().len() as u64, buf);
        buf.extend_from_slice(blob.as_slice());
    }
}

fn encode_batch_sync_request(buf: &mut Vec<u8>, req: &BatchSyncRequest) {
    buf.extend_from_slice(req.id.as_bytes());
    buf.extend_from_slice(req.req_id.requestor.as_bytes());
    buf.extend_from_slice(&req.req_id.nonce.to_be_bytes());
    buf.push(u8::from(req.subscribe));

    let seed = req.fingerprint_summary.seed();
    buf.extend_from_slice(&seed.key0().to_be_bytes());
    buf.extend_from_slice(&seed.key1().to_be_bytes());

    let commit_fps = req.fingerprint_summary.commit_fingerprints();
    let fragment_fps = req.fingerprint_summary.fragment_fingerprints();
    #[allow(clippy::cast_possible_truncation)]
    {
        buf.extend_from_slice(&(commit_fps.len() as u16).to_be_bytes());
        buf.extend_from_slice(&(fragment_fps.len() as u16).to_be_bytes());
    }

    for fp in commit_fps {
        buf.extend_from_slice(&fp.as_u64().to_be_bytes());
    }
    for fp in fragment_fps {
        buf.extend_from_slice(&fp.as_u64().to_be_bytes());
    }
}

fn encode_batch_sync_response(buf: &mut Vec<u8>, resp: &BatchSyncResponse) {
    buf.extend_from_slice(resp.req_id.requestor.as_bytes());
    buf.extend_from_slice(&resp.req_id.nonce.to_be_bytes());
    buf.extend_from_slice(resp.id.as_bytes());

    match &resp.result {
        SyncResult::Ok(diff) => {
            buf.push(result_tags::OK);
            encode_sync_diff(buf, diff);
        }
        SyncResult::NotFound => {
            buf.push(result_tags::NOT_FOUND);
        }
        SyncResult::Unauthorized => {
            buf.push(result_tags::UNAUTHORIZED);
        }
    }

    encode_remote_heads(buf, &resp.responder_heads);
}

fn encode_sync_diff(buf: &mut Vec<u8>, diff: &SyncDiff) {
    #[allow(clippy::cast_possible_truncation)]
    {
        buf.extend_from_slice(&(diff.missing_commits.len() as u16).to_be_bytes());
        buf.extend_from_slice(&(diff.missing_fragments.len() as u16).to_be_bytes());
        buf.extend_from_slice(&(diff.requesting.commit_fingerprints.len() as u16).to_be_bytes());
        buf.extend_from_slice(
            &(diff.requesting.fragment_fingerprints.len() as u16).to_be_bytes(),
        );
    }

    for (signed, blob) in &diff.missing_commits {
        buf.extend_from_slice(signed.as_bytes());
        bijou64::encode(blob.as_slice().len() as u64, buf);
        buf.extend_from_slice(blob.as_slice());
    }

    for (signed, blob) in &diff.missing_fragments {
        buf.extend_from_slice(signed.as_bytes());
        bijou64::encode(blob.as_slice().len() as u64, buf);
        buf.extend_from_slice(blob.as_slice());
    }

    for fp in &diff.requesting.commit_fingerprints {
        buf.extend_from_slice(&fp.as_u64().to_be_bytes());
    }
    for fp in &diff.requesting.fragment_fingerprints {
        buf.extend_from_slice(&fp.as_u64().to_be_bytes());
    }
}

fn encode_remove_subscriptions(buf: &mut Vec<u8>, unsub: &RemoveSubscriptions) {
    #[allow(clippy::cast_possible_truncation)]
    buf.extend_from_slice(&(unsub.ids.len() as u16).to_be_bytes());
    for id in &unsub.ids {
        buf.extend_from_slice(id.as_bytes());
    }
}

fn encode_data_request_rejected(buf: &mut Vec<u8>, rejected: &DataRequestRejected) {
    buf.extend_from_slice(rejected.id.as_bytes());
}

// ============================================================================
// Decode
// ============================================================================

#[allow(clippy::indexing_slicing)] // Length validated before access
fn decode_message(bytes: &[u8]) -> Result<SyncMessage, DecodeError> {
    if bytes.len() < ENVELOPE_HEADER_SIZE {
        return Err(DecodeError::MessageTooShort {
            type_name: "Message envelope",
            need: ENVELOPE_HEADER_SIZE,
            have: bytes.len(),
        });
    }

    let schema: [u8; 4] =
        bytes
            .get(0..4)
            .and_then(|s| s.try_into().ok())
            .ok_or(DecodeError::MessageTooShort {
                type_name: "Message schema",
                need: 4,
                have: bytes.len(),
            })?;
    if schema != MESSAGE_SCHEMA {
        return Err(InvalidSchema {
            expected: MESSAGE_SCHEMA,
            got: schema,
        }
        .into());
    }

    let total_size = u32::from_be_bytes(bytes.get(4..8).and_then(|s| s.try_into().ok()).ok_or(
        DecodeError::MessageTooShort {
            type_name: "Message total_size",
            need: 8,
            have: bytes.len(),
        },
    )?) as usize;
    if bytes.len() != total_size {
        return Err(SizeMismatch {
            declared: total_size,
            actual: bytes.len(),
        }
        .into());
    }

    let tag = *bytes.get(8).ok_or(DecodeError::MessageTooShort {
        type_name: "Message tag",
        need: 9,
        have: bytes.len(),
    })?;
    let payload = bytes
        .get(ENVELOPE_HEADER_SIZE..)
        .ok_or(DecodeError::MessageTooShort {
            type_name: "Message payload",
            need: ENVELOPE_HEADER_SIZE,
            have: bytes.len(),
        })?;

    let (min_payload_size, type_name) = match tag {
        tags::LOOSE_COMMIT => (min_sizes::LOOSE_COMMIT, "LooseCommit"),
        tags::FRAGMENT => (min_sizes::FRAGMENT, "Fragment"),
        tags::BLOBS_REQUEST => (min_sizes::BLOBS_REQUEST, "BlobsRequest"),
        tags::BLOBS_RESPONSE => (min_sizes::BLOBS_RESPONSE, "BlobsResponse"),
        tags::BATCH_SYNC_REQUEST => (min_sizes::BATCH_SYNC_REQUEST, "BatchSyncRequest"),
        tags::BATCH_SYNC_RESPONSE => (min_sizes::BATCH_SYNC_RESPONSE, "BatchSyncResponse"),
        tags::REMOVE_SUBSCRIPTIONS => (min_sizes::REMOVE_SUBSCRIPTIONS, "RemoveSubscriptions"),
        tags::DATA_REQUEST_REJECTED => (min_sizes::DATA_REQUEST_REJECTED, "DataRequestRejected"),
        tags::HEADS_UPDATE => (min_sizes::HEADS_UPDATE, "HeadsUpdate"),
        _ => {
            return Err(InvalidEnumTag {
                tag,
                type_name: "Message",
            }
            .into());
        }
    };

    if payload.len() < min_payload_size {
        return Err(DecodeError::MessageTooShort {
            type_name,
            need: ENVELOPE_HEADER_SIZE + min_payload_size,
            have: bytes.len(),
        });
    }

    match tag {
        tags::LOOSE_COMMIT => decode_loose_commit(payload),
        tags::FRAGMENT => decode_fragment(payload),
        tags::BLOBS_REQUEST => decode_blobs_request(payload),
        tags::BLOBS_RESPONSE => decode_blobs_response(payload),
        tags::BATCH_SYNC_REQUEST => decode_batch_sync_request(payload),
        tags::BATCH_SYNC_RESPONSE => decode_batch_sync_response(payload),
        tags::REMOVE_SUBSCRIPTIONS => decode_remove_subscriptions(payload),
        tags::DATA_REQUEST_REJECTED => decode_data_request_rejected(payload),
        tags::HEADS_UPDATE => decode_heads_update(payload),
        _ => unreachable!("tag validated above"),
    }
}

fn decode_remote_heads(payload: &[u8], offset: &mut usize) -> Result<RemoteHeads, DecodeError> {
    let counter = read_u64(payload, offset)?;
    let count = read_u32(payload, offset)? as usize;
    // Cap allocation based on remaining payload to prevent OOM from untrusted input.
    let remaining = payload.len().saturating_sub(*offset);
    let capacity = core::cmp::min(count, remaining / 32);
    let mut heads = Vec::with_capacity(capacity);
    for _ in 0..count {
        heads.push(Digest::force_from_bytes(read_array::<32>(payload, offset)?));
    }
    Ok(RemoteHeads { counter, heads })
}

fn decode_loose_commit(payload: &[u8]) -> Result<SyncMessage, DecodeError> {
    let mut offset = 0;

    let id = SedimentreeId::new(read_array::<32>(payload, &mut offset)?);
    let sender_heads = decode_remote_heads(payload, &mut offset)?;

    let commit = Signed::<LooseCommit>::try_decode(
        payload
            .get(offset..)
            .ok_or(BufferTooShort {
                reading: ReadingType::Slice { len: 0 },
                offset,
                need: 1,
                have: 0,
            })?
            .to_vec(),
    )?;
    offset += commit.as_bytes().len();

    let blob_size = read_bijou64_as_usize(payload, &mut offset)?;

    let blob = Blob::new(
        payload
            .get(offset..offset + blob_size)
            .ok_or(BufferTooShort {
                reading: ReadingType::Slice { len: blob_size },
                offset,
                need: blob_size,
                have: payload.len().saturating_sub(offset),
            })?
            .to_vec(),
    );

    Ok(SyncMessage::LooseCommit {
        id,
        commit,
        blob,
        sender_heads,
    })
}

fn decode_fragment(payload: &[u8]) -> Result<SyncMessage, DecodeError> {
    let mut offset = 0;

    let id = SedimentreeId::new(read_array::<32>(payload, &mut offset)?);
    let sender_heads = decode_remote_heads(payload, &mut offset)?;

    let fragment = Signed::<Fragment>::try_decode(
        payload
            .get(offset..)
            .ok_or(BufferTooShort {
                reading: ReadingType::Slice { len: 0 },
                offset,
                need: 1,
                have: 0,
            })?
            .to_vec(),
    )?;
    offset += fragment.as_bytes().len();

    let blob_size = read_bijou64_as_usize(payload, &mut offset)?;

    let blob = Blob::new(
        payload
            .get(offset..offset + blob_size)
            .ok_or(BufferTooShort {
                reading: ReadingType::Slice { len: blob_size },
                offset,
                need: blob_size,
                have: payload.len().saturating_sub(offset),
            })?
            .to_vec(),
    );

    Ok(SyncMessage::Fragment {
        id,
        fragment,
        blob,
        sender_heads,
    })
}

fn decode_blobs_request(payload: &[u8]) -> Result<SyncMessage, DecodeError> {
    let mut offset = 0;

    let id = SedimentreeId::new(read_array::<32>(payload, &mut offset)?);
    let count = read_u16(payload, &mut offset)? as usize;

    let mut digests = Vec::with_capacity(count);
    for _ in 0..count {
        digests.push(Digest::force_from_bytes(read_array::<32>(
            payload,
            &mut offset,
        )?));
    }

    Ok(SyncMessage::BlobsRequest { id, digests })
}

fn decode_blobs_response(payload: &[u8]) -> Result<SyncMessage, DecodeError> {
    let mut offset = 0;

    let id = SedimentreeId::new(read_array::<32>(payload, &mut offset)?);
    let count = read_u16(payload, &mut offset)? as usize;

    let mut blobs = Vec::with_capacity(count);
    for _ in 0..count {
        let blob_size = read_bijou64_as_usize(payload, &mut offset)?;
        blobs.push(Blob::new(
            payload
                .get(offset..offset + blob_size)
                .ok_or(BufferTooShort {
                    reading: ReadingType::Slice { len: blob_size },
                    offset,
                    need: blob_size,
                    have: payload.len().saturating_sub(offset),
                })?
                .to_vec(),
        ));
        offset += blob_size;
    }

    Ok(SyncMessage::BlobsResponse { id, blobs })
}

fn decode_batch_sync_request(payload: &[u8]) -> Result<SyncMessage, DecodeError> {
    let mut offset = 0;

    let id = SedimentreeId::new(read_array::<32>(payload, &mut offset)?);

    let requestor = PeerId::new(read_array::<32>(payload, &mut offset)?);
    let nonce = read_u64(payload, &mut offset)?;
    let req_id = RequestId { requestor, nonce };

    let subscribe_byte = read_u8(payload, &mut offset)?;
    let subscribe = match subscribe_byte {
        0x00 => false,
        0x01 => true,
        _ => {
            return Err(InvalidEnumTag {
                tag: subscribe_byte,
                type_name: "Subscribe",
            }
            .into());
        }
    };

    let key0 = read_u64(payload, &mut offset)?;
    let key1 = read_u64(payload, &mut offset)?;
    let seed = FingerprintSeed::new(key0, key1);

    let commit_count = read_u16(payload, &mut offset)? as usize;
    let fragment_count = read_u16(payload, &mut offset)? as usize;

    let mut commit_fps = BTreeSet::new();
    for _ in 0..commit_count {
        commit_fps.insert(Fingerprint::from_u64(read_u64(payload, &mut offset)?));
    }
    let mut fragment_fps = BTreeSet::new();
    for _ in 0..fragment_count {
        fragment_fps.insert(Fingerprint::from_u64(read_u64(payload, &mut offset)?));
    }

    let fingerprint_summary = FingerprintSummary::new(seed, commit_fps, fragment_fps);

    Ok(SyncMessage::BatchSyncRequest(BatchSyncRequest {
        id,
        req_id,
        fingerprint_summary,
        subscribe,
    }))
}

fn decode_batch_sync_response(payload: &[u8]) -> Result<SyncMessage, DecodeError> {
    let mut offset = 0;

    let requestor = PeerId::new(read_array::<32>(payload, &mut offset)?);
    let nonce = read_u64(payload, &mut offset)?;
    let req_id = RequestId { requestor, nonce };

    let id = SedimentreeId::new(read_array::<32>(payload, &mut offset)?);

    let result_tag = read_u8(payload, &mut offset)?;
    let result = match result_tag {
        result_tags::OK => SyncResult::Ok(decode_sync_diff(payload, &mut offset)?),
        result_tags::NOT_FOUND => SyncResult::NotFound,
        result_tags::UNAUTHORIZED => SyncResult::Unauthorized,
        _ => {
            return Err(InvalidEnumTag {
                tag: result_tag,
                type_name: "SyncResult",
            }
            .into());
        }
    };

    let responder_heads = decode_remote_heads(payload, &mut offset)?;

    Ok(SyncMessage::BatchSyncResponse(BatchSyncResponse {
        req_id,
        id,
        result,
        responder_heads,
    }))
}

fn decode_sync_diff(payload: &[u8], offset: &mut usize) -> Result<SyncDiff, DecodeError> {
    let commit_count = read_u16(payload, offset)? as usize;
    let fragment_count = read_u16(payload, offset)? as usize;
    let requested_commit_count = read_u16(payload, offset)? as usize;
    let requested_fragment_count = read_u16(payload, offset)? as usize;

    let mut missing_commits = Vec::with_capacity(commit_count);
    for _ in 0..commit_count {
        let commit = Signed::<LooseCommit>::try_decode(
            payload
                .get(*offset..)
                .ok_or(BufferTooShort {
                    reading: ReadingType::Slice { len: 0 },
                    offset: *offset,
                    need: 1,
                    have: 0,
                })?
                .to_vec(),
        )?;
        *offset += commit.as_bytes().len();

        let blob_size = read_bijou64_as_usize(payload, offset)?;
        let blob = Blob::new(
            payload
                .get(*offset..*offset + blob_size)
                .ok_or(BufferTooShort {
                    reading: ReadingType::Slice { len: blob_size },
                    offset: *offset,
                    need: blob_size,
                    have: payload.len().saturating_sub(*offset),
                })?
                .to_vec(),
        );
        *offset += blob_size;

        missing_commits.push((commit, blob));
    }

    let mut missing_fragments = Vec::with_capacity(fragment_count);
    for _ in 0..fragment_count {
        let fragment = Signed::<Fragment>::try_decode(
            payload
                .get(*offset..)
                .ok_or(BufferTooShort {
                    reading: ReadingType::Slice { len: 0 },
                    offset: *offset,
                    need: 1,
                    have: 0,
                })?
                .to_vec(),
        )?;
        *offset += fragment.as_bytes().len();

        let blob_size = read_bijou64_as_usize(payload, offset)?;
        let blob = Blob::new(
            payload
                .get(*offset..*offset + blob_size)
                .ok_or(BufferTooShort {
                    reading: ReadingType::Slice { len: blob_size },
                    offset: *offset,
                    need: blob_size,
                    have: payload.len().saturating_sub(*offset),
                })?
                .to_vec(),
        );
        *offset += blob_size;

        missing_fragments.push((fragment, blob));
    }

    let mut commit_fingerprints = Vec::with_capacity(requested_commit_count);
    for _ in 0..requested_commit_count {
        commit_fingerprints.push(Fingerprint::from_u64(read_u64(payload, offset)?));
    }
    let mut fragment_fingerprints = Vec::with_capacity(requested_fragment_count);
    for _ in 0..requested_fragment_count {
        fragment_fingerprints.push(Fingerprint::from_u64(read_u64(payload, offset)?));
    }

    Ok(SyncDiff {
        missing_commits,
        missing_fragments,
        requesting: RequestedData {
            commit_fingerprints,
            fragment_fingerprints,
        },
    })
}

fn decode_remove_subscriptions(payload: &[u8]) -> Result<SyncMessage, DecodeError> {
    let mut offset = 0;

    let count = read_u16(payload, &mut offset)? as usize;

    let mut ids = Vec::with_capacity(count);
    for _ in 0..count {
        ids.push(SedimentreeId::new(read_array::<32>(payload, &mut offset)?));
    }

    Ok(SyncMessage::RemoveSubscriptions(RemoveSubscriptions {
        ids,
    }))
}

fn decode_heads_update(payload: &[u8]) -> Result<SyncMessage, DecodeError> {
    let mut offset = 0;

    let id = SedimentreeId::new(read_array::<32>(payload, &mut offset)?);
    let heads = decode_remote_heads(payload, &mut offset)?;

    Ok(SyncMessage::HeadsUpdate { id, heads })
}

fn decode_data_request_rejected(payload: &[u8]) -> Result<SyncMessage, DecodeError> {
    let mut offset = 0;

    let id = SedimentreeId::new(read_array::<32>(payload, &mut offset)?);

    Ok(SyncMessage::DataRequestRejected(DataRequestRejected { id }))
}

// ============================================================================
// Reader helpers
// ============================================================================

fn read_u8(buf: &[u8], offset: &mut usize) -> Result<u8, DecodeError> {
    let val = *buf.get(*offset).ok_or(BufferTooShort {
        reading: ReadingType::U8,
        offset: *offset,
        need: 1,
        have: buf.len().saturating_sub(*offset),
    })?;
    *offset += 1;
    Ok(val)
}

fn read_u16(buf: &[u8], offset: &mut usize) -> Result<u16, DecodeError> {
    let val = u16::from_be_bytes(
        buf.get(*offset..*offset + 2)
            .and_then(|s| s.try_into().ok())
            .ok_or(BufferTooShort {
                reading: ReadingType::U16,
                offset: *offset,
                need: 2,
                have: buf.len().saturating_sub(*offset),
            })?,
    );
    *offset += 2;
    Ok(val)
}

fn read_u32(buf: &[u8], offset: &mut usize) -> Result<u32, DecodeError> {
    let val = u32::from_be_bytes(
        buf.get(*offset..*offset + 4)
            .and_then(|s| s.try_into().ok())
            .ok_or(BufferTooShort {
                reading: ReadingType::U32,
                offset: *offset,
                need: 4,
                have: buf.len().saturating_sub(*offset),
            })?,
    );
    *offset += 4;
    Ok(val)
}

fn read_u64(buf: &[u8], offset: &mut usize) -> Result<u64, DecodeError> {
    let val = u64::from_be_bytes(
        buf.get(*offset..*offset + 8)
            .and_then(|s| s.try_into().ok())
            .ok_or(BufferTooShort {
                reading: ReadingType::U64,
                offset: *offset,
                need: 8,
                have: buf.len().saturating_sub(*offset),
            })?,
    );
    *offset += 8;
    Ok(val)
}

fn read_array<const N: usize>(buf: &[u8], offset: &mut usize) -> Result<[u8; N], DecodeError> {
    let arr: [u8; N] = buf
        .get(*offset..*offset + N)
        .and_then(|s| s.try_into().ok())
        .ok_or(BufferTooShort {
            reading: ReadingType::Array { size: N },
            offset: *offset,
            need: N,
            have: buf.len().saturating_sub(*offset),
        })?;
    *offset += N;
    Ok(arr)
}

fn read_bijou64(buf: &[u8], offset: &mut usize) -> Result<u64, DecodeError> {
    let (val, consumed) =
        bijou64::decode(buf.get(*offset..).unwrap_or_default()).map_err(|kind| Bijou64Error {
            offset: *offset,
            kind,
        })?;
    *offset += consumed;
    Ok(val)
}

fn read_bijou64_as_usize(buf: &[u8], offset: &mut usize) -> Result<usize, DecodeError> {
    let val = read_bijou64(buf, offset)?;
    usize::try_from(val).map_err(|_| {
        BlobTooLarge {
            size: val,
            max: usize::MAX as u64,
        }
        .into()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn invalid_schema_rejected() {
        let mut bad_bytes = vec![0x00; 20];
        bad_bytes[0..4].copy_from_slice(b"BAD\x00");
        bad_bytes[4..8].copy_from_slice(&20u32.to_be_bytes());

        let result = SyncMessage::try_decode(&bad_bytes);
        assert!(matches!(result, Err(DecodeError::InvalidSchema(_))));
    }

    #[test]
    fn size_mismatch_rejected() {
        let msg = SyncMessage::DataRequestRejected(DataRequestRejected {
            id: SedimentreeId::new([42u8; 32]),
        });
        let mut encoded = msg.encode();
        encoded.truncate(encoded.len() - 5);

        let result = SyncMessage::try_decode(&encoded);
        assert!(matches!(result, Err(DecodeError::SizeMismatch(_))));
    }

    #[test]
    fn invalid_tag_rejected() {
        let mut bad_bytes = vec![0x00; 20];
        bad_bytes[0..4].copy_from_slice(&MESSAGE_SCHEMA);
        bad_bytes[4..8].copy_from_slice(&20u32.to_be_bytes());
        bad_bytes[8] = 0xFF;

        let result = SyncMessage::try_decode(&bad_bytes);
        assert!(matches!(result, Err(DecodeError::InvalidEnumTag(_))));
    }

    #[test]
    fn data_request_rejected_roundtrip() {
        let msg = SyncMessage::DataRequestRejected(DataRequestRejected {
            id: SedimentreeId::new([7u8; 32]),
        });
        let encoded = msg.encode();
        let decoded = SyncMessage::try_decode(&encoded).expect("decode should succeed");
        assert_eq!(msg, decoded);
    }

    #[test]
    fn remove_subscriptions_roundtrip() {
        let msg = SyncMessage::RemoveSubscriptions(RemoveSubscriptions {
            ids: vec![
                SedimentreeId::new([1u8; 32]),
                SedimentreeId::new([2u8; 32]),
            ],
        });
        let encoded = msg.encode();
        let decoded = SyncMessage::try_decode(&encoded).expect("decode should succeed");
        assert_eq!(msg, decoded);
    }

    #[test]
    fn blobs_request_roundtrip() {
        let msg = SyncMessage::BlobsRequest {
            id: SedimentreeId::new([3u8; 32]),
            digests: vec![
                Digest::force_from_bytes([10u8; 32]),
                Digest::force_from_bytes([20u8; 32]),
            ],
        };
        let encoded = msg.encode();
        let decoded = SyncMessage::try_decode(&encoded).expect("decode should succeed");
        assert_eq!(msg, decoded);
    }

    #[test]
    fn blobs_response_roundtrip() {
        let msg = SyncMessage::BlobsResponse {
            id: SedimentreeId::new([4u8; 32]),
            blobs: vec![Blob::new(vec![1, 2, 3]), Blob::new(vec![4, 5, 6, 7])],
        };
        let encoded = msg.encode();
        let decoded = SyncMessage::try_decode(&encoded).expect("decode should succeed");
        assert_eq!(msg, decoded);
    }

    #[test]
    fn heads_update_roundtrip() {
        let msg = SyncMessage::HeadsUpdate {
            id: SedimentreeId::new([5u8; 32]),
            heads: RemoteHeads {
                counter: 42,
                heads: vec![Digest::force_from_bytes([99u8; 32])],
            },
        };
        let encoded = msg.encode();
        let decoded = SyncMessage::try_decode(&encoded).expect("decode should succeed");
        assert_eq!(msg, decoded);
    }

    #[test]
    fn batch_sync_request_roundtrip() {
        let seed = FingerprintSeed::new(0x1234, 0x5678);
        let mut commit_fps = BTreeSet::new();
        commit_fps.insert(Fingerprint::from_u64(111));
        let mut fragment_fps = BTreeSet::new();
        fragment_fps.insert(Fingerprint::from_u64(222));

        let msg = SyncMessage::BatchSyncRequest(BatchSyncRequest {
            id: SedimentreeId::new([6u8; 32]),
            req_id: RequestId {
                requestor: PeerId::new([11u8; 32]),
                nonce: 99,
            },
            fingerprint_summary: FingerprintSummary::new(seed, commit_fps, fragment_fps),
            subscribe: true,
        });
        let encoded = msg.encode();
        let decoded = SyncMessage::try_decode(&encoded).expect("decode should succeed");
        assert_eq!(msg, decoded);
    }

    #[test]
    fn batch_sync_response_not_found_roundtrip() {
        let msg = SyncMessage::BatchSyncResponse(BatchSyncResponse {
            req_id: RequestId {
                requestor: PeerId::new([12u8; 32]),
                nonce: 77,
            },
            id: SedimentreeId::new([8u8; 32]),
            result: SyncResult::NotFound,
            responder_heads: RemoteHeads {
                counter: 1,
                heads: vec![],
            },
        });
        let encoded = msg.encode();
        let decoded = SyncMessage::try_decode(&encoded).expect("decode should succeed");
        assert_eq!(msg, decoded);
    }

    #[test]
    fn batch_sync_response_unauthorized_roundtrip() {
        let msg = SyncMessage::BatchSyncResponse(BatchSyncResponse {
            req_id: RequestId {
                requestor: PeerId::new([13u8; 32]),
                nonce: 88,
            },
            id: SedimentreeId::new([9u8; 32]),
            result: SyncResult::Unauthorized,
            responder_heads: RemoteHeads {
                counter: 2,
                heads: vec![Digest::force_from_bytes([55u8; 32])],
            },
        });
        let encoded = msg.encode();
        let decoded = SyncMessage::try_decode(&encoded).expect("decode should succeed");
        assert_eq!(msg, decoded);
    }

    #[test]
    fn envelope_header_is_sum_null() {
        let msg = SyncMessage::DataRequestRejected(DataRequestRejected {
            id: SedimentreeId::new([0u8; 32]),
        });
        let encoded = msg.encode();
        assert_eq!(&encoded[0..4], b"SUM\x00");
    }

    #[test]
    fn envelope_total_size_matches() {
        let msg = SyncMessage::DataRequestRejected(DataRequestRejected {
            id: SedimentreeId::new([0u8; 32]),
        });
        let encoded = msg.encode();
        let declared = u32::from_be_bytes(encoded[4..8].try_into().unwrap()) as usize;
        assert_eq!(declared, encoded.len());
    }

    // ------- bolero round-trip tests -------

    fn arb_peer_id(u: &mut arbitrary::Unstructured) -> arbitrary::Result<PeerId> {
        Ok(PeerId::new(u.arbitrary()?))
    }

    fn arb_remote_heads(u: &mut arbitrary::Unstructured) -> arbitrary::Result<RemoteHeads> {
        let counter: u64 = u.arbitrary()?;
        let count: u8 = u.arbitrary()?;
        let mut heads = Vec::with_capacity(count as usize);
        for _ in 0..count {
            heads.push(Digest::force_from_bytes(u.arbitrary()?));
        }
        Ok(RemoteHeads { counter, heads })
    }

    fn arb_request_id(u: &mut arbitrary::Unstructured) -> arbitrary::Result<RequestId> {
        Ok(RequestId {
            requestor: arb_peer_id(u)?,
            nonce: u.arbitrary()?,
        })
    }

    fn arb_batch_sync_request(
        u: &mut arbitrary::Unstructured,
    ) -> arbitrary::Result<SyncMessage> {
        let id: SedimentreeId = u.arbitrary()?;
        let req_id = arb_request_id(u)?;
        let subscribe: bool = u.arbitrary()?;
        let seed = FingerprintSeed::new(u.arbitrary()?, u.arbitrary()?);
        let commit_count: u8 = u.int_in_range(0..=16)?;
        let fragment_count: u8 = u.int_in_range(0..=16)?;
        let mut commit_fps = BTreeSet::new();
        for _ in 0..commit_count {
            commit_fps.insert(Fingerprint::from_u64(u.arbitrary()?));
        }
        let mut fragment_fps = BTreeSet::new();
        for _ in 0..fragment_count {
            fragment_fps.insert(Fingerprint::from_u64(u.arbitrary()?));
        }
        Ok(SyncMessage::BatchSyncRequest(BatchSyncRequest {
            id,
            req_id,
            fingerprint_summary: FingerprintSummary::new(seed, commit_fps, fragment_fps),
            subscribe,
        }))
    }

    fn arb_blobs_request(
        u: &mut arbitrary::Unstructured,
    ) -> arbitrary::Result<SyncMessage> {
        let id: SedimentreeId = u.arbitrary()?;
        let count: u8 = u.int_in_range(0..=8)?;
        let mut digests = Vec::with_capacity(count as usize);
        for _ in 0..count {
            digests.push(Digest::force_from_bytes(u.arbitrary()?));
        }
        Ok(SyncMessage::BlobsRequest { id, digests })
    }

    fn arb_blobs_response(
        u: &mut arbitrary::Unstructured,
    ) -> arbitrary::Result<SyncMessage> {
        let id: SedimentreeId = u.arbitrary()?;
        let count: u8 = u.int_in_range(0..=4)?;
        let mut blobs = Vec::with_capacity(count as usize);
        for _ in 0..count {
            let len: u8 = u.int_in_range(0..=64)?;
            let data: Vec<u8> = (0..len).map(|_| u.arbitrary()).collect::<arbitrary::Result<_>>()?;
            blobs.push(Blob::new(data));
        }
        Ok(SyncMessage::BlobsResponse { id, blobs })
    }

    fn arb_remove_subscriptions(
        u: &mut arbitrary::Unstructured,
    ) -> arbitrary::Result<SyncMessage> {
        let count: u8 = u.int_in_range(0..=8)?;
        let mut ids = Vec::with_capacity(count as usize);
        for _ in 0..count {
            ids.push(SedimentreeId::new(u.arbitrary()?));
        }
        Ok(SyncMessage::RemoveSubscriptions(RemoveSubscriptions { ids }))
    }

    fn arb_heads_update(
        u: &mut arbitrary::Unstructured,
    ) -> arbitrary::Result<SyncMessage> {
        Ok(SyncMessage::HeadsUpdate {
            id: u.arbitrary()?,
            heads: arb_remote_heads(u)?,
        })
    }

    fn arb_data_request_rejected(
        u: &mut arbitrary::Unstructured,
    ) -> arbitrary::Result<SyncMessage> {
        Ok(SyncMessage::DataRequestRejected(DataRequestRejected {
            id: u.arbitrary()?,
        }))
    }

    fn arb_batch_sync_response_simple(
        u: &mut arbitrary::Unstructured,
    ) -> arbitrary::Result<SyncMessage> {
        let req_id = arb_request_id(u)?;
        let id: SedimentreeId = u.arbitrary()?;
        let result_tag: u8 = u.int_in_range(1..=2)?;
        let result = match result_tag {
            1 => SyncResult::NotFound,
            _ => SyncResult::Unauthorized,
        };
        Ok(SyncMessage::BatchSyncResponse(BatchSyncResponse {
            req_id,
            id,
            result,
            responder_heads: arb_remote_heads(u)?,
        }))
    }

    /// Generate an arbitrary simple SyncMessage (no Signed<T> variants).
    fn arb_simple_message(
        u: &mut arbitrary::Unstructured,
    ) -> arbitrary::Result<SyncMessage> {
        let variant: u8 = u.int_in_range(0..=5)?;
        match variant {
            0 => arb_batch_sync_request(u),
            1 => arb_blobs_request(u),
            2 => arb_blobs_response(u),
            3 => arb_remove_subscriptions(u),
            4 => arb_heads_update(u),
            5 => arb_data_request_rejected(u),
            _ => arb_batch_sync_response_simple(u),
        }
    }

    #[test]
    fn bolero_simple_message_roundtrip() {
        bolero::check!().with_arbitrary::<[u8; 256]>().for_each(|bytes| {
            let mut u = arbitrary::Unstructured::new(bytes);
            let Ok(msg) = arb_simple_message(&mut u) else {
                return;
            };
            let encoded = msg.encode();
            let decoded = SyncMessage::try_decode(&encoded).expect("roundtrip decode should succeed");
            assert_eq!(msg, decoded);
        });
    }

    #[test]
    fn bolero_batch_sync_response_simple_roundtrip() {
        bolero::check!().with_arbitrary::<[u8; 128]>().for_each(|bytes| {
            let mut u = arbitrary::Unstructured::new(bytes);
            let Ok(msg) = arb_batch_sync_response_simple(&mut u) else {
                return;
            };
            let encoded = msg.encode();
            let decoded = SyncMessage::try_decode(&encoded).expect("roundtrip decode should succeed");
            assert_eq!(msg, decoded);
        });
    }
}
