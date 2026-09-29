# Changelog

## Unreleased

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
