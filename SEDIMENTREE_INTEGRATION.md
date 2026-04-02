# Sedimentree Integration

## Problem

Subduction sync relies on sedimentree, which organizes data into **fragments** (chunks of commits grouped by boundary detection) and **loose commits** (individual depth-0 changes). Currently, every automerge change sent to subduction stays as a LooseCommit forever — no fragments are created. This means batch sync fingerprinting can't leverage the hierarchical structure sedimentree is designed for.

## Background

### Sedimentree data model

Sedimentree uses a probabilistic chunking strategy based on commit hash prefixes:

- **Depth metric** (`CountLeadingZeroBytes`): counts leading zero bytes in the commit digest (which equals the automerge change hash bytes). Depth 0 = no leading zeros, depth 1 = one leading zero byte (~1/256 probability), depth 2 = two leading zero bytes (~1/65536).
- **Loose commits**: depth-0 changes that aren't covered by any fragment. Each wraps a single automerge change as a blob.
- **Fragments**: chunks headed by a depth > 0 commit. A fragment collects all commits reachable from its head with depth ≤ the head's depth, stopping at commits with higher depth (which form the fragment's boundary). The fragment's blob is the concatenation of all member changes' raw bytes in topological order.

### Existing machinery

The `automerge_sedimentree` crate provides `ingest_automerge(&doc, sed_id)` which takes a full `Automerge` document reference and produces an `IngestResult` containing:
- A `Sedimentree` (fragments + loose commits with blob metadata)
- All corresponding `Blob`s
- Accounting info (change count, covered count, loose count, fragment count)

The ingest pipeline has two phases:
1. **Metadata phase** (cheap): `get_changes_meta()` extracts the DAG structure, then `build_fragment_store()` computes fragment boundaries and membership via BFS.
2. **Byte extraction phase** (expensive): `get_changes()` reconstructs raw change bytes, which are grouped by fragment membership and concatenated into blobs.

## Approach

Run `ingest_automerge` in the document actor after changes are made, diff the result against previously-sent sedimentree state, and send new fragments + loose commits to the hub. The hub forwards them to the subduction engine for signing, storage, and sync.

The document actor is the natural home for this because it owns the `Automerge` doc, which `ingest_automerge` requires.

### Performance considerations

- **Most changes are depth-0** (~255/256) and won't create new fragments. We can check cheaply by inspecting change hashes for leading zero bytes before running the full ingest.
- **The metadata phase is cheap.** If no new boundary commits exist, we can skip the expensive byte extraction phase entirely.
- **Diffing avoids re-sending.** Since `ingest_automerge` processes the entire doc, we track which fragment/commit digests have already been sent and only send new ones.

## Implementation steps

### 1. Add `automerge_sedimentree` dependency to samod-core

**File**: `samod-core/Cargo.toml`

Add as an optional dependency gated behind the `subduction` feature:
```toml
automerge_sedimentree = { path = "../../subduction/automerge_sedimentree", optional = true }
```

### 2. Replace `SubductionChangeInfo` with sedimentree ingest data

**File**: `samod-core/src/actors/messages/doc_to_hub_msg.rs`

Instead of sending individual `SubductionChangeInfo` per change, the doc actor sends the ingest result. New message payload:

```rust
#[cfg(feature = "subduction")]
NewSedimentreeData {
    document_id: DocumentId,
    /// New fragments (not previously sent)
    fragments: Vec<(Fragment, Blob)>,
    /// New loose commits (not previously sent)
    loose_commits: Vec<(LooseCommit, Blob)>,
}
```

### 3. Run `ingest_automerge` in the document actor's save path

**File**: `samod-core/src/actors/document/on_disk_state.rs`

In `save_new_changes`, replace the current `SubductionChangeInfo` extraction with:

1. Call `ingest_automerge(doc, sed_id)` to produce the full sedimentree
2. Diff against previously-sent state (track sent fragment/commit digests in `OnDiskState`)
3. Send only new fragments and loose commits via `NewSedimentreeData`

### 4. Add `Input::NewSedimentreeData` to the subduction engine

**File**: `subduction-sans-io/src/engine.rs`

New input variant:

```rust
NewSedimentreeData {
    sed_id: SedimentreeId,
    fragments: Vec<(Fragment, Blob)>,
    loose_commits: Vec<(LooseCommit, Blob)>,
}
```

The handler stores each blob (fire-and-forget) and requests signing for each fragment and loose commit. `Input::NewLocalChanges` can remain for other consumers of the engine.

### 5. Add `PendingSign::FragmentSign` to the engine

**File**: `subduction-sans-io/src/engine.rs`

New variant alongside the existing `CommitSign`:

```rust
FragmentSign { sed_id: SedimentreeId, fragment: Fragment, blob: Blob }
```

In `handle_signing_complete`, handle `FragmentSign` by creating `Signed<Fragment>`, storing via `storage::fragments_to_kv_ops`, and pushing to subscribers.

### 6. Add `IncrementalSync::on_local_fragment`

**File**: `subduction-sans-io/src/incremental.rs`

Mirror of `on_local_commit`, using `SyncMessage::Fragment` (which already exists in the wire protocol).

### 7. Update the hub to forward the new message type

**Files**:
- `samod-core/src/actors/hub/subduction_sync.rs` — replace `on_new_changes_from_doc` with `on_new_sedimentree_data` that converts the doc actor's message into `Input::NewSedimentreeData`
- `samod-core/src/actors/hub/state.rs` — replace the `NewChangesForSubduction` match arm with `NewSedimentreeData`

## Key files

| File | Role |
|------|------|
| `samod-core/Cargo.toml` | Add `automerge_sedimentree` dep |
| `samod-core/src/actors/document/on_disk_state.rs` | Run ingest, diff, send |
| `samod-core/src/actors/messages/doc_to_hub_msg.rs` | New message type |
| `samod-core/src/actors/hub/subduction_sync.rs` | Forward to engine |
| `samod-core/src/actors/hub/state.rs` | Handle new message |
| `subduction-sans-io/src/engine.rs` | New input + fragment signing |
| `subduction-sans-io/src/incremental.rs` | `on_local_fragment` |

## Verification

1. `cargo test -p samod-core` — existing tests pass without feature flag
2. `cargo test -p samod-core --features subduction` — all tests pass with feature flag
3. `cargo test -p samod-core --features subduction --test subduction_sync` — subduction sync tests pass
4. New test: create a document with ~500 changes, sync via subduction, verify the receiving peer gets fragment data (not just loose commits)
