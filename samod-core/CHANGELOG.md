# Changelog

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
