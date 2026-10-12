# Changelog

## Unreleased

### Breaking Changes

- Replaced `DocumentIoTask::CheckAnnouncePolicy` and
  `DocumentIoResult::CheckAnnouncePolicy(bool)` with `CheckSharePolicy` tasks and
  `DocumentIoResult::CheckSharePolicy(ShareConfig)`.
- Added `ShareConfig` and `ShareAccess` to control read/write access and
  announcements together. `PeerDocState` includes `share_config` and
  `peer_read_only`; `PeerRequestState` includes `BlockedByPolicy`.

### Added

- Enforce read-only sync with Automerge's read-only sync state. Denied peers
  receive document-unavailable responses without disclosing document existence.
- Apply sharing permissions to ephemeral delivery and gossip. Read-only peers
  retain ephemeral access; denied peers do not.
- Exclude policy-blocked sources from document lookup and preserve read-only
  access when resetting sync state.
- `HubEvent::update_share_policy(document_id, peer_id, config)` installs sharing
  decisions on active document/peer connections without issuing new policy checks.
  Pending initial checks are superseded so late results cannot undo an update.

### Fixed

- Avoid sync stalls after access changes when reset support is unknown or absent,
  including when a modern peer's first reply was rejected during denial. Keep
  the Automerge workaround local to sync message generation.

## 0.15.0 - 2026-09-30

### Breaking Changes

- `HubEvent::create_dialer_connection` and `create_listener_connection` now take
  an `expected_peer_id: Option<PeerId>`. Pass `None` to trust the handshake's
  claimed identity, or `Some(peer_id)` to bind an authenticated transport to it.

### Fixed

- Reject mismatched handshake identities, inconsistent direct-message senders,
  and messages targeting another peer before dispatching them.
- Protocol and decode failures emit disconnect IO and connection-failure events,
  notify document actors, and update listener membership and dialer retry state.

## 0.14.0 - 2026-09-17

### Breaking Changes

- Updated to `automerge@0.12.0`.
- `HubEvent::dial_failed` now takes a `permanent` boolean indicating whether
  retrying can succeed.

### Changed

- Permanent dial failures transition the dialer directly to its terminal
  `Failed` state without applying backoff or consuming the retry budget.

## 0.13.0 - 2026-08-12

### Breaking Changes

- Updated to `automerge@0.11.0`.

## 0.12.2 - 2026-07-08

Released to stay in lockstep with the `samod` crate; no library changes.

## 0.12.0 - 2026-06-05

### Breaking changes

- Updated to `automerge@0.10.0`.

## 0.11.0 - 2026-06-04

### Breaking changes

- Replaced the `HubEvent::find_document` command constructor with
  `HubEvent::search_for_doc` to support the new document search state API.
- Replaced `CommandResult::FindDocument { actor_id, found }` with
  `CommandResult::SearchForDoc { actor_id, search_state }`. Callers must now
  inspect the returned `DocSearch` state instead of a boolean `found` flag.
- Added the public `HubResults::search_state_updates` field. Code that
  constructs or exhaustively destructures `HubResults` must be updated to
  include this field or use `..`.
