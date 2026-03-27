# Subduction Sans-IO: Design Document

## Overview

This document describes how to implement the [subduction sync protocol](../../subduction/design/) in samod using a sans-IO architecture. The goal is to add subduction as an **additional sync protocol option** alongside the existing automerge sync protocol, while preserving samod-core's event-driven, IO-free state machine design.

Both protocols run simultaneously on the same samod instance. Connections are tagged at creation time as either automerge-sync or subduction connections — there is no multiplexing of protocols on a single connection. This allows us to adopt subduction incrementally while retaining the existing automerge sync protocol's capabilities (notably transitive request forwarding, which subduction does not yet support).

We reuse `sedimentree_core` (and `automerge_sedimentree`) from the subduction workspace as dependencies. The protocol state machines, message encoding, and handshake are implemented from scratch using samod's sans-IO patterns rather than porting subduction's async/await + future-form approach.

## Motivation

Subduction's main advantage over the existing sync protocol is that it does not require loading the automerge document to perform sync. This is useful because loading automerge documents can be computationally intensive (hence the need to run each document actor on it's own thread). 

However, the existing automerge sync protocol still has value:
- **Transitive request forwarding** — peers can request documents they don't have, and the request is forwarded through the network. Subduction does not yet support this.
- **JS interoperability** — the automerge-repo wire protocol enables interop with JavaScript clients.

By running both protocols simultaneously, a samod instance can use subduction for peers that support it while falling back to automerge sync for JS interop and transitive forwarding.

## Architecture

### New Crate: `subduction-sans-io`

A new crate in the samod workspace, sitting alongside `samod-core`:

```
samod/
├── samod-core/          # Existing: Hub, DocumentActor, IO abstractions
├── samod/               # Existing: Async runtime wrapper
├── subduction-sans-io/  # NEW: Sans-IO subduction protocol
│   └── src/
│       ├── lib.rs
│       ├── handshake.rs       # Handshake state machine
│       ├── batch_sync.rs      # Batch sync state machine
│       ├── incremental.rs     # Incremental sync (push) logic
│       ├── subscriptions.rs   # Subscription tracking
│       ├── messages.rs        # Protocol message types + codec
│       ├── connection.rs      # Per-connection state
│       ├── remote_heads.rs    # Remote heads tracking with monotonic counters
│       └── peer_counter.rs    # Per-peer monotonic counter
└── samod-test-harness/  # Existing: Test utilities
```

### Dependency Graph

```
subduction-sans-io
├── sedimentree_core        (from ../subduction/sedimentree_core)
├── automerge_sedimentree   (from ../subduction/automerge_sedimentree)
├── subduction_crypto       (from ../subduction/subduction_crypto — Ed25519 types, Signed<T>)
└── bijou64                 (from ../subduction/bijou64 — canonical varint encoding)

samod-core
├── subduction-sans-io      (new dependency)
├── automerge
└── (existing deps)
```

We depend directly on `subduction_crypto`, `bijou64`, and other subduction crates rather than vendoring them. This keeps us in sync with upstream and avoids duplication. We can vendor later if independence from upstream changes becomes important.

`SedimentreeId` maps directly to `DocumentId` — a 1:1 relationship with no indirection layer. Each automerge document corresponds to exactly one sedimentree.

Sedimentree blobs are stored separately from automerge's incremental/snapshot storage. This means duplication of change data, but cleaner abstraction and less coupling between the two storage layers.

## Key Design Decisions

### 1. Sans-IO Over Async/Await

Subduction uses `async fn` + `FutureForm` for FFI flexibility. We instead use samod's established pattern:

```rust
// Subduction (async):
async fn sync_with_peer(&self, peer_id: PeerId) -> Result<()> {
    let request = self.build_request(sedimentree_id).await;
    let response = connection.call(request).await?;
    self.process_response(response).await?;
    Ok(())
}

// subduction-sans-io (event-driven):
fn handle_event(&mut self, event: SubductionEvent) -> SubductionResult {
    match event {
        SubductionEvent::InitiateSync { peer_id, sedimentree_id } => {
            let request = self.build_request(sedimentree_id);
            result.send_message(peer_id, request);
        }
        SubductionEvent::MessageReceived { peer_id, message } => {
            match message {
                SyncMessage::BatchSyncResponse(resp) => {
                    self.process_response(peer_id, resp, &mut result);
                }
                // ...
            }
        }
    }
    result
}
```

The caller (samod-core's Hub) drives the state machine by feeding events and executing the returned IO tasks. Since subduction sync is IO-bound (not CPU-bound like automerge document loading), it runs directly in the Hub's event loop.

### 2. Dual-Protocol Connections

Connections are classified at creation time. The Hub tracks which protocol each connection uses:

```rust
pub enum ConnectionProtocol {
    /// Existing automerge-repo wire protocol (CBOR, Join/Peer handshake)
    AutomergeSync,
    /// Subduction protocol (binary codec, Ed25519 handshake)
    Subduction,
}
```

This is configured by the caller when creating dialers/listeners — e.g., a dialer targeting a known subduction peer uses `ConnectionProtocol::Subduction`, while a dialer targeting a JS automerge-repo peer uses `ConnectionProtocol::AutomergeSync`. There is no protocol detection or negotiation on the wire.

### 3. Integration Point: Hub, Not DocumentActor

The key architectural insight is that **subduction sync does not require loading the automerge document**. It only needs access to sedimentree metadata, which is small and fast to parse. This means subduction sync should live in the Hub, not in the DocumentActor.

This is a significant advantage: automerge documents can be computationally expensive to load (which is why each DocumentActor runs on its own thread). Subduction sync is IO-bound (storage reads/writes, network sends), not CPU-bound, so it can run directly in the Hub's event loop without blocking.

```
Hub (central coordinator)
├── (existing) automerge-sync connection handling → routes to DocumentActors
├── (NEW) SubductionSync                          ← handles subduction directly
│   ├── sedimentrees: HashMap<DocumentId, Sedimentree>  (loaded from storage, not from doc)
│   ├── peer_states: HashMap<ConnectionId, SubductionPeerState>
│   ├── subscriptions: SubscriptionTracker
│   ├── pending_requests: HashMap<RequestId, PendingBatchSync>
│   └── incremental: IncrementalSync
└── (existing) DocumentActors                     ← only for automerge sync + doc mutations

DocumentActor (existing, mostly unchanged)
├── doc_state: DocState (existing phases)
├── peer_connections: HashMap<ConnectionId, PeerDocConnection>  ← automerge sync only
└── on_disk_state: OnDiskState
```

**Data flow between Hub and DocumentActors:**

```
Hub (subduction)                          DocumentActor
       │                                        │
       │  DocToHubMsg::NewSubductionData         │
       │  { commits, fragments, blobs }          │
       │───────────────────────────────────────→  │  (actor applies blobs to automerge doc)
       │                                        │
       │  HubToDocMsg::LocalChangeNotify         │
       │←───────────────────────────────────────  │  (actor notifies hub of local changes)
       │                                        │
       │  (hub pushes to subduction subscribers) │
```

When subduction receives new data from a peer:
1. Hub stores the sedimentree metadata (commits/fragments) and blobs to storage
2. Hub sends blobs to the DocumentActor so it can apply them to the automerge document
3. DocumentActor applies changes, which also triggers automerge-sync messages to automerge-sync peers

When a DocumentActor has new local changes (from local edits or automerge-sync):
1. DocumentActor notifies Hub of the new changes
2. Hub builds sedimentree metadata from the new changes (signed commits + blobs)
3. Hub stores the new sedimentree data
4. Hub pushes incremental updates to subscribed subduction peers

### 4. Handshake: Protocol-Dependent

Each protocol has its own handshake, determined by the connection's protocol tag:

- **AutomergeSync connections:** Use the existing `Join`/`Peer` CBOR handshake (unchanged)
- **Subduction connections:** Use Ed25519 challenge-response handshake

```
Hub
├── connections: HashMap<ConnectionId, ConnectionInfo>
│   └── ConnectionInfo {
│         protocol: ConnectionProtocol,
│         state: ConnectionState,  // Handshaking or Connected
│       }
├── subduction_sync: SubductionSync  ← NEW (all subduction state lives here)
│   ├── handshakes: HashMap<ConnectionId, HandshakeStateMachine>
│   ├── nonce_cache: NonceCache
│   ├── sedimentrees: HashMap<DocumentId, Sedimentree>
│   ├── subscriptions: SubscriptionTracker
│   ├── doc_requests: HashMap<DocumentId, SubductionDocRequestState>
│   └── ... (signing is async IO, not held in state)
└── (existing state)
```

### 5. Storage Model Changes

Sedimentree data requires new storage keys alongside existing automerge storage:

```
Current (automerge sync):
  {doc_id}/snapshot/{compaction_hash}
  {doc_id}/incremental/{change_hash}

New (subduction):
  {doc_id}/sedimentree/fragment/{fragment_digest}      # Fragment metadata (signed)
  {doc_id}/sedimentree/commit/{commit_digest}          # LooseCommit metadata (signed)
  {doc_id}/sedimentree/blob/{blob_digest}              # Content-addressed blobs
  {doc_id}/sedimentree/meta                            # Cached sedimentree summary
```

We keep the existing automerge storage for document content. The sedimentree layer stores blobs separately — this duplicates change data but keeps the two storage layers cleanly decoupled. We can optimize to deduplicate later if needed.

## Protocol State Machines

### Handshake State Machine

For subduction connections only. Automerge-sync connections continue to use the existing `Join`/`Peer` handshake.

```
                    ┌──────────┐
                    │  Start   │
                    └────┬─────┘
                         │
              ┌──────────┴──────────┐
              │                     │
        (we initiate)         (they initiate)
              │                     │
    ┌─────────▼─────────┐  ┌───────▼────────────┐
    │ AwaitingResponse   │  │ ReceivedChallenge   │
    │ (sent Challenge)   │  │ (verify + respond)  │
    └─────────┬─────────┘  └───────┬────────────┘
              │                     │
    ┌─────────▼─────────┐          │
    │ VerifyResponse     │          │
    └─────────┬─────────┘          │
              │                     │
              └──────────┬──────────┘
                         │
                ┌────────▼────────┐
                │  Authenticated  │
                │  { peer_id }    │
                └─────────────────┘
```

**Input events:**
- `HandshakeEvent::Start { we_initiate: bool }`
- `HandshakeEvent::MessageReceived { bytes: Vec<u8> }`

**Output actions:**
- `HandshakeAction::SendMessage { bytes: Vec<u8> }`
- `HandshakeAction::Completed { peer_id: PeerId }`
- `HandshakeAction::Failed { reason: HandshakeError }`

**IO requirements:**
- Signing (Ed25519) — async IO task (see "Async Batch Signing" section below).
- Nonce generation — requires RNG, passed in as parameter (consistent with samod's `&mut R: Rng` pattern).
- Timestamp — passed in as `now: UnixTimestamp` parameter.

Note: the handshake only needs to sign one message per direction, so the batching benefit is minimal here. But we use the same async signing interface for consistency.

### Batch Sync State Machine

Manages one batch sync exchange for a single sedimentree between two peers.

```
    ┌──────────────┐
    │  Initiating   │  (compute fingerprints, send request)
    └──────┬───────┘
           │ SendBatchSyncRequest
           ▼
    ┌──────────────┐
    │  AwaitResp    │  (waiting for BatchSyncResponse)
    └──────┬───────┘
           │ ReceiveBatchSyncResponse
           ▼
    ┌──────────────────┐
    │  Processing       │  (store received data, resolve requesting fingerprints)
    │  Response         │
    └──────┬───────────┘
           │ SendRequestedItems (fire-and-forget)
           ▼
    ┌──────────────┐
    │  Complete     │
    └──────────────┘
```

**As responder:**

```
    ReceiveBatchSyncRequest
           │
           ▼
    ┌───────────────────┐
    │  LoadLocal         │  (load local sedimentree — may need storage IO)
    └──────┬────────────┘
           │ StorageComplete
           ▼
    ┌───────────────────┐
    │  ComputeDiff       │  (fingerprint diff, gather missing data)
    └──────┬────────────┘
           │ SendBatchSyncResponse
           ▼
    ┌───────────────────┐
    │  AwaitFireForget   │  (receive requested items, if any)
    └──────┬────────────┘
           │ (items received or timeout)
           ▼
    ┌──────────────┐
    │  Complete     │
    └──────────────┘
```

**Key difference from subduction:** In subduction, `sync_with_peer` is a single async function that awaits the response. In sans-IO, we split this into discrete states and handle the response as a separate event. The `RequestId` correlates requests to responses.

**IO requirements:**
- `StorageTask::LoadSedimentree { doc_id }` — load local sedimentree metadata
- `StorageTask::LoadBlobs { digests }` — load blob data for items to send
- `StorageTask::PutCommit { commit, blob }` — store received commit + blob
- `StorageTask::PutFragment { fragment, blob }` — store received fragment + blob

### Incremental Sync (Subscription Push)

Not a traditional state machine — more of a reactive system:

```rust
/// Tracks subscriptions and generates push messages
pub struct IncrementalSync {
    /// Peers subscribed to each sedimentree
    subscriptions: HashMap<SedimentreeId, HashSet<PeerId>>,
    /// Per-peer monotonic counters
    peer_counters: HashMap<PeerId, u64>,
}

impl IncrementalSync {
    /// Called when a local change is made to a document
    fn on_local_change(
        &mut self,
        sedimentree_id: SedimentreeId,
        commit: Signed<LooseCommit>,
        blob: Vec<u8>,
    ) -> Vec<(PeerId, SyncMessage)> {
        // For each subscribed peer, generate a LooseCommit message
        // with incremented per-peer counter
    }

    /// Called when a forwarded change arrives from a peer
    fn on_remote_change(
        &mut self,
        from_peer: PeerId,
        sedimentree_id: SedimentreeId,
        commit: Signed<LooseCommit>,
        blob: Vec<u8>,
    ) -> Vec<(PeerId, SyncMessage)> {
        // Forward to all subscribed peers EXCEPT the sender
        // (avoids echo)
    }
}
```

### Remote Heads Tracking

Tracks peer heads with monotonic counters to filter stale updates:

```rust
pub struct RemoteHeadsTracker {
    /// Last seen counter per peer
    last_counter: HashMap<PeerId, u64>,
}

impl RemoteHeadsTracker {
    /// Returns Some(heads) if this update is fresh, None if stale
    fn update(&mut self, peer: PeerId, counter: u64, heads: Vec<Digest>) -> Option<Vec<Digest>> {
        if counter > *self.last_counter.get(&peer).unwrap_or(&0) {
            self.last_counter.insert(peer, counter);
            Some(heads)
        } else {
            None
        }
    }
}
```

## Async Batch Signing

Signing is modeled as an async IO task, not a synchronous in-process call. While we can implement a simple in-memory Ed25519 signer initially, the interface must be designed for the long-term case where signing is performed by an external service (HSM, remote signer, etc.) that may have latency.

**Design principle: always be batching.** The signing interface accepts multiple items to sign at once, amortizing any per-call overhead (network round trip to HSM, key unlock, etc.).

```rust
/// IO task for batch signing
pub enum SigningIoTask {
    /// Sign a batch of payloads. Each item is an opaque byte buffer to be signed.
    /// The response returns signatures in the same order.
    SignBatch {
        items: Vec<SigningRequest>,
    },
}

pub struct SigningRequest {
    /// Unique ID to correlate request with response
    id: SigningRequestId,
    /// The bytes to sign (canonical encoding of the payload)
    payload: Vec<u8>,
}

pub enum SigningIoResult {
    SignBatch {
        signatures: Vec<(SigningRequestId, Ed25519Signature)>,
    },
}
```

**Where signing happens:**

The state machines in `subduction-sans-io` are purely synchronous — they never call a signer directly. Instead, when they need something signed, they return a `SigningIoTask` as part of their result. The caller (the Hub) collects pending signing requests, batches them, and issues a single `SignBatch` IO task. When the result comes back, the Hub feeds the signatures back into the state machines.

This means state machines that need signing have an intermediate state: "waiting for signature". For example, the handshake state machine goes:

```
Start → BuildChallenge → AwaitingSignature → SendChallenge → AwaitingResponse → ...
```

**Batching opportunities:**
- When a DocumentActor notifies the Hub of N new changes, the Hub needs to sign N `LooseCommit` entries — this is a natural batch.
- When the Hub needs to forward incremental changes to multiple peers, the commits are already signed (signatures are part of the `Signed<T>` wrapper), so no additional signing is needed for forwarding.
- Handshake signing is inherently single-item, but uses the same interface.

## Request Lifecycle with Subduction

The document request lifecycle (finding a document you don't have locally) must account for both protocols. A document should not be declared `NotFound` until both automerge-sync *and* subduction have exhausted their options.

### Current Request Flow (automerge-sync only)

```
DocumentActor spawned → Loading phase
    ↓ (load from automerge storage)
    ↓ empty?
    ↓
Requesting phase
    ├── For each automerge-sync peer: send Request, track state
    ├── Wait for: any peer to respond with data → Ready
    ├── Wait for: pending dialer connections (don't give up while a dialer is connecting)
    └── All peers unavailable AND no dialers connecting → NotFound
```

The key completion check in `Request::status()`:
- `finished = all_peers_unavailable && !any_dialer_connecting`
- `found = !all_unavailable && any_sync_is_done`

### Extended Request Flow (dual protocol)

With subduction, there are two additional checks before declaring `NotFound`:
1. **Is there sedimentree data in storage?** — The Hub can check this without loading the automerge document.
2. **Have we requested from subduction peers?** — The Hub needs to try batch sync with connected subduction peers.

These checks happen in the Hub (where subduction lives), but the request lifecycle decision (Ready vs NotFound) happens in the DocumentActor. We need coordination between them.

**Proposed flow:**

```
App calls find(doc_id)
    │
    ▼
Hub spawns DocumentActor                   Hub checks subduction storage
    │                                           │
    ▼                                           ▼
DocumentActor: Loading phase               SubductionSync: check for sedimentree
(load automerge storage)                   (load sedimentree metadata from storage)
    │                                           │
    ▼                                           ▼
Automerge storage empty?                   Sedimentree found in storage?
    │                                           │
    ├── YES: enter Requesting phase             ├── YES: initiate batch sync with
    │        (automerge-sync peers)             │        subduction peers to get
    │                                           │        latest data, send blobs
    │                                           │        to DocumentActor
    │                                           │
    │                                           ├── NO: request from connected
    │                                           │       subduction peers
    │                                           │       (BatchSyncRequest)
    │                                           │
    │                                           └── Wait for pending subduction
    │                                                dialer connections
    ▼
DocumentActor Requesting phase:
    Check: is subduction still looking? ──────── Hub reports subduction request status
    │                                            (via new message type)
    ├── any AM-sync peer found it → Ready
    ├── subduction found it (blobs arrive) → Ready
    ├── all AM-sync peers unavailable
    │   AND subduction exhausted → NotFound
    └── otherwise → keep waiting
```

### Hub ↔ DocumentActor Coordination for Requests

The Hub needs to tell the DocumentActor about subduction's request status. We introduce a new message:

```rust
pub enum HubToDocMsgPayload {
    // ... existing variants ...

    /// Update on subduction's attempt to find this document.
    SubductionRequestStatus {
        status: SubductionRequestStatus,
    },
}

pub enum SubductionRequestStatus {
    /// Subduction is still looking (checking storage, waiting for peers,
    /// waiting for pending connections)
    Searching,
    /// Subduction found the document — blobs will follow via ApplySubductionData
    Found,
    /// Subduction has exhausted all options (no sedimentree in storage,
    /// all subduction peers responded NotFound, no pending subduction connections)
    NotFound,
}
```

The DocumentActor's `Request::status()` method is extended to account for subduction:

```rust
pub(crate) fn status(
    &self,
    doc: &Automerge,
    any_dialer_connecting: bool,
    subduction_status: SubductionRequestStatus,  // NEW
) -> RequestState {
    let all_am_peers_unavailable = /* ... existing logic ... */;
    let all_am_unavailable = all_am_peers_unavailable && !any_dialer_connecting;

    let subduction_exhausted = matches!(
        subduction_status,
        SubductionRequestStatus::NotFound
    );
    let subduction_found = matches!(
        subduction_status,
        SubductionRequestStatus::Found
    );

    let any_sync_is_done = /* ... existing logic ... */ || subduction_found;

    // Don't declare NotFound until BOTH protocols have exhausted their options
    let all_exhausted = all_am_unavailable && subduction_exhausted;

    RequestState {
        finished: all_exhausted || any_sync_is_done,
        found: any_sync_is_done,
    }
}
```

### Subduction Request State Machine (in Hub)

The Hub tracks a per-document subduction request state:

```rust
pub enum SubductionDocRequestState {
    /// Checking if sedimentree metadata exists in storage
    CheckingStorage,
    /// Requesting from connected subduction peers (BatchSyncRequest sent)
    RequestingFromPeers {
        /// Peers we've sent BatchSyncRequest to
        requested: HashSet<ConnectionId>,
        /// Peers that responded NotFound
        not_found: HashSet<ConnectionId>,
        /// Whether any subduction dialer is still connecting
        any_dialer_connecting: bool,
    },
    /// Found — batch sync succeeded, blobs sent to DocumentActor
    Found,
    /// All avenues exhausted
    NotFound,
}
```

This mirrors the structure of the existing `Request` phase in the DocumentActor but operates at the Hub level with subduction peers.

**Completion logic:**
- `Found` when: batch sync returns data for this document from any peer, OR sedimentree metadata exists in storage and sync completes successfully
- `NotFound` when: no sedimentree in storage AND all subduction peers respond `NotFound` AND no subduction dialers are connecting
- Keep `Searching` while: any subduction peer hasn't responded yet, or a subduction dialer is still connecting

### Timing: When Does the Hub Start the Subduction Request?

The Hub starts the subduction lookup as soon as `find(doc_id)` is called — it doesn't wait for the DocumentActor's Loading phase to complete. This means the subduction storage check and peer requests happen **in parallel** with the automerge storage load, potentially finding the document faster.

```
Time ──────────────────────────────────────────────────────→

DocumentActor:  [  Loading (automerge storage)  ] → Requesting → ...
Hub/Subduction: [  Check sedimentree storage  ] → [  BatchSync with peers  ] → ...
                ↑
                Both start immediately when find() is called
```

## Integration with samod-core

### Hub: Subduction Sync Owner

The Hub becomes the owner of all subduction sync state. Subduction messages on subduction connections are handled entirely within the Hub — they never reach DocumentActors. The Hub loads sedimentree metadata from storage, performs fingerprint diffs, sends/receives sync messages, and stores incoming data — all without loading any automerge document.

**Hub subduction responsibilities:**
1. **Handshake** — manage Ed25519 challenge-response for subduction connections
2. **Batch sync** — handle `BatchSyncRequest`/`BatchSyncResponse` using sedimentree metadata from storage
3. **Incremental sync** — push new commits/fragments to subscribed peers, receive pushes from peers
4. **Subscription management** — track which peers are subscribed to which sedimentrees
5. **Sedimentree storage** — load/save sedimentree metadata and blobs via storage IO tasks

```rust
// New/modified fields in Hub state
pub struct HubState {
    // ... existing fields (connections, dialers, listeners, etc.) ...

    // Subduction-specific (all optional — not present if subduction unconfigured)
    subduction: Option<SubductionSync>,
}

pub struct SubductionSync {
    nonce_cache: NonceCache,
    handshakes: HashMap<ConnectionId, HandshakeStateMachine>,
    /// Sedimentree metadata per document, loaded from storage (not from automerge doc)
    sedimentrees: HashMap<DocumentId, Sedimentree>,
    /// Per-connection peer sync state
    peer_states: HashMap<ConnectionId, SubductionPeerState>,
    /// Subscription tracking
    subscriptions: SubscriptionTracker,
    /// Pending batch sync requests (waiting for responses)
    pending_requests: HashMap<RequestId, PendingBatchSync>,
    /// Incremental sync (push) state
    incremental: IncrementalSync,
    /// Per-document request state (for documents we're trying to find)
    doc_requests: HashMap<DocumentId, SubductionDocRequestState>,
    /// Pending signing requests (waiting for async signer response)
    pending_signatures: HashMap<SigningRequestId, PendingSignatureContext>,
}
```

### DocumentActor: Mostly Unchanged

DocumentActors continue to handle automerge-sync connections exactly as before. They do **not** know about subduction connections or the subduction protocol. Two new message types bridge the protocols:

**Hub → DocumentActor (new data from subduction):**

When the Hub receives new blobs via subduction, it needs the DocumentActor to apply them to the automerge document:

```rust
pub enum HubToDocMsgPayload {
    // ... existing variants ...

    /// New data arrived via subduction — apply these blobs to the automerge doc
    ApplySubductionData {
        /// Blobs in topsorted dependency order, ready for load_incremental()
        blobs: Vec<Vec<u8>>,
    },
}
```

**DocumentActor → Hub (local changes for subduction):**

When the DocumentActor has new changes (from local edits or automerge-sync), it notifies the Hub so subduction can push them:

```rust
pub enum DocToHubMsgPayload {
    // ... existing variants ...

    /// New local changes that need to be propagated via subduction.
    /// Contains the raw automerge changes so the Hub can build sedimentree
    /// commits and blobs from them.
    NewChangesForSubduction {
        changes: Vec<(ChangeHash, Vec<u8>)>,  // hash + raw change bytes
    },
}
```

The Hub receives this, builds `Signed<LooseCommit>` entries and blobs, updates the in-memory sedimentree, stores to disk, and pushes to subscribed subduction peers.

### Cross-Protocol Change Propagation

```
Subduction peer                Hub                    DocumentActor           AM-sync peer
     │                          │                          │                      │
     │ LooseCommit(blob)        │                          │                      │
     │─────────────────────────→│                          │                      │
     │                          │ store sedimentree data   │                      │
     │                          │ to storage               │                      │
     │                          │                          │                      │
     │                          │ ApplySubductionData      │                      │
     │                          │─────────────────────────→│                      │
     │                          │                          │ load_incremental()   │
     │                          │                          │ generate_sync_msg()  │
     │                          │                          │─────────────────────→│
     │                          │                          │                      │
```

```
AM-sync peer              DocumentActor                Hub                 Subduction peer
     │                          │                          │                      │
     │ SyncMessage(changes)     │                          │                      │
     │─────────────────────────→│                          │                      │
     │                          │ receive_sync_message()   │                      │
     │                          │                          │                      │
     │                          │ NewChangesForSubduction  │                      │
     │                          │─────────────────────────→│                      │
     │                          │                          │ build commits+blobs  │
     │                          │                          │ store sedimentree    │
     │                          │                          │ push to subscribers  │
     │                          │                          │─────────────────────→│
     │                          │                          │                      │
```

### New IO Task Types

The Hub issues its own IO tasks for subduction storage. These are separate from DocumentActor IO tasks:

```rust
pub enum SubductionIoTask {
    /// Load the sedimentree metadata for a document
    LoadSedimentree { doc_id: DocumentId },

    /// Load specific blobs by digest
    LoadBlobs { doc_id: DocumentId, digests: Vec<Digest<Blob>> },

    /// Store a received commit and its blob
    PutCommit {
        doc_id: DocumentId,
        commit: Signed<LooseCommit>,
        blob: Vec<u8>,
    },

    /// Store a received fragment and its blob
    PutFragment {
        doc_id: DocumentId,
        fragment: Signed<Fragment>,
        blob: Vec<u8>,
    },

    /// Sign a batch of payloads. Always batch — even single items go
    /// through this path for interface consistency.
    SignBatch {
        items: Vec<SigningRequest>,
    },
}
```

These are issued as part of `HubResults` (alongside the existing `HubIoAction::Send`/`Disconnect`), following samod-core's `IoTask<Action>` / `IoResult<Payload>` pattern. The runtime executes them and feeds results back via `HubEvent`.

Note there is no `ApplyBlobs` IO task here — applying blobs to the automerge document is handled by sending an `ApplySubductionData` message to the DocumentActor, which handles it in-process.

### Wire Protocol: Two Protocols, Separate Connections

Since each connection is tagged with its protocol at creation time, there is no need for protocol negotiation or multiplexing on the wire:

- **AutomergeSync connections** use the existing CBOR-based `WireMessage` format (unchanged)
- **Subduction connections** use subduction's binary codec with `SUH\0` (handshake) and `SUM\0` (sync message) envelopes

The Hub routes incoming bytes to the appropriate handler based on the connection's protocol tag:
- AutomergeSync → existing message routing to DocumentActors
- Subduction → handled directly in the Hub's `SubductionSync`

## Message Types

The sans-IO crate defines its own message types mirroring subduction's protocol, with encode/decode using the canonical binary codec:

```rust
pub enum SyncMessage {
    BatchSyncRequest {
        id: SedimentreeId,
        req_id: RequestId,
        fingerprint_summary: FingerprintSummary,
        subscribe: bool,
    },
    BatchSyncResponse {
        req_id: RequestId,
        id: SedimentreeId,
        result: SyncResult,
    },
    LooseCommit {
        id: SedimentreeId,
        commit: Signed<LooseCommit>,
        blob: Vec<u8>,
        sender_heads: RemoteHeads,
    },
    Fragment {
        id: SedimentreeId,
        fragment: Signed<Fragment>,
        blob: Vec<u8>,
        sender_heads: RemoteHeads,
    },
    HeadsUpdate {
        id: SedimentreeId,
        heads: RemoteHeads,
    },
    BlobsRequest {
        id: SedimentreeId,
        digests: Vec<Digest<Blob>>,
    },
    BlobsResponse {
        id: SedimentreeId,
        blobs: Vec<(Digest<Blob>, Vec<u8>)>,
    },
    RemoveSubscriptions {
        ids: Vec<SedimentreeId>,
    },
}

pub enum SyncResult {
    Ok(SyncDiff),
    NotFound,
    Unauthorized,
}

pub struct SyncDiff {
    pub missing_commits: Vec<(Signed<LooseCommit>, Vec<u8>)>,
    pub missing_fragments: Vec<(Signed<Fragment>, Vec<u8>)>,
    pub requesting: FingerprintSummary,  // what responder needs from us
    pub responder_heads: RemoteHeads,
}
```

## Sedimentree ↔ Automerge Bridge

The `automerge_sedimentree` crate provides the bridge between automerge documents and sedimentree metadata. Key operations:

1. **Building a sedimentree from an automerge doc:**
   ```
   Automerge → CommitStore → build_fragment_store() → Sedimentree + blobs
   ```

2. **Applying received sedimentree data to an automerge doc:**
   ```
   Received blobs (topsorted) → automerge::load_incremental() for each
   ```

3. **Computing fingerprints for batch sync:**
   ```
   Sedimentree → fingerprint_summarize(seed) → FingerprintSummary
   ```

4. **Computing diffs:**
   ```
   local_sedimentree.diff(&remote_fingerprints, seed) → missing items
   ```

### Document ID ↔ Sedimentree ID Mapping

`SedimentreeId` maps directly to `DocumentId` — a 1:1 relationship. Each automerge document corresponds to exactly one sedimentree. We use a direct cast or trivial conversion between the two types.

## Phased Implementation Plan

### Phase 1: Foundation

1. **Create `subduction-sans-io` crate** with workspace dependencies on `sedimentree_core`, `subduction_crypto`, `bijou64`
2. **Implement message types** (`messages.rs`) with canonical binary encode/decode
3. **Implement handshake state machine** (`handshake.rs`) — pure state machine, no IO
4. **Unit tests** for message round-tripping and handshake state transitions

### Phase 2: Batch Sync

1. **Implement batch sync state machines** (initiator + responder) in `batch_sync.rs`
2. **Implement sedimentree storage** — new `StorageKey` variants, load/save operations
3. **Implement the automerge ↔ sedimentree bridge** integration for building sedimentrees from loaded automerge docs
4. **Integration test** with two in-memory peers doing batch sync

### Phase 3: Incremental Sync

1. **Implement subscription tracking** (`subscriptions.rs`)
2. **Implement incremental push** (`incremental.rs`)
3. **Implement remote heads tracking** (`remote_heads.rs`)
4. **Integration test** with subscription-based live updates

### Phase 4: samod-core Integration (Dual Protocol)

1. **Add `ConnectionProtocol` enum** and extend connection tracking in Hub
2. **Add `SubductionSync` to Hub** — owns all subduction state, handles subduction connections directly
3. **Add subduction handshake** to Hub for subduction-tagged connections (automerge-sync unchanged)
4. **Route incoming bytes by protocol** — automerge-sync → DocumentActors (as before), subduction → Hub's SubductionSync
5. **Add `SubductionIoTask`** to `HubResults` for sedimentree storage operations
6. **Add `ApplySubductionData`** message (Hub → DocumentActor) for applying received blobs
7. **Add `NewChangesForSubduction`** message (DocumentActor → Hub) for propagating local changes
8. **End-to-end test** with a samod instance using both automerge-sync and subduction connections simultaneously

### Phase 5: samod Wrapper Integration

1. **Update `io_loop`** to execute new IO task types (sedimentree storage)
2. **Add optional `Signer` configuration** to `Repo` builder (subduction is opt-in)
3. **Extend dialer/listener config** with `ConnectionProtocol` selection
4. **Integration tests** with mixed-protocol network topology (some peers subduction, some automerge-sync)

## Open Questions

1. **Authorization/Policy:** Subduction has `StoragePolicy` with `authorize_put` and `filter_authorized_fetch`. Samod has `AnnouncePolicy`. How do these map? The announce policy is simpler (bool per doc×peer). We may need to extend it or introduce a new policy trait. For now, the announce policy could apply to both protocols — it already gates per-doc×peer visibility.

2. **Signing key management:** Where does the Ed25519 signing key come from? Options: generate and store alongside `StorageId`, derive from `StorageId`, or require application to provide it. The async signing interface means the key doesn't need to live in-process.

3. **Fragment creation:** When does a peer create fragments from loose commits? On every save? Periodically? On compaction? This affects storage size and sync efficiency.

4. **Sedimentree bootstrapping:** When subduction is first enabled on an instance with existing automerge documents, we need to build sedimentree metadata from the existing automerge change history. This could happen lazily (on first subduction sync) or eagerly (on startup). Lazy is simpler but adds latency to the first sync.

5. **Multi-document batch sync:** Subduction supports syncing multiple sedimentrees in a single batch request. Should we support this, or keep it one-document-at-a-time? Since subduction lives in the Hub (not per-document actors), multi-document batch sync is architecturally feasible and potentially more efficient.

6. **Request forwarding gap:** Since subduction doesn't support transitive request forwarding, a document that only exists on peers reachable via subduction connections cannot be discovered through automerge-sync request forwarding (and vice versa). Is this acceptable, or do we need the Hub to bridge request forwarding across protocols? For now, the assumption is that both protocol types have independent peer sets and the application configures connections appropriately.

7. **Connection protocol selection UX:** How does the application decide which protocol to use for a given connection? Hardcoded per-dialer? Configuration? Peer capability discovery? For now, this is explicit in dialer/listener configuration.

8. **Sedimentree lifecycle in Hub:** The Hub holds in-memory sedimentree metadata for documents being synced via subduction. When should these be loaded (on first subduction sync request? eagerly for all known documents?) and when should they be evicted (LRU? when no subduction peers are subscribed?)? Since sedimentree metadata is small, we could keep all known sedimentrees in memory, but this doesn't scale indefinitely.

9. **NewChangesForSubduction message content:** When the DocumentActor notifies the Hub of new local changes, what exactly should it send? Options: (a) raw change bytes + automerge metadata (parents, hash) so the Hub builds LooseCommits, or (b) pre-built unsigned LooseCommit payloads ready for signing. Option (a) is simpler for the DocumentActor; option (b) avoids the Hub needing to understand automerge change structure.
