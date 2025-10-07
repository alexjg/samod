# Changelog

## 0.5.0 - 2025-10-03

### Changed

* `AutomergeUrl` is now clone (thanks to @pkgw)

### Breaking Changes

* Updated to `automerge` 0.7.0

## 0.4.0 - 2025-09-15

### Added

* It is now possible to use `Storage` and `AnnouncePolicy` implementations which
  can't support `Send` futures. This is enabled by implementing the new
  `LocalStorage` and `LocalAnnouncePolicy` traits instead of `Storage` and
  `AnnouncePolicy`. and loading the repo using a `LocalRuntimeHandle` rather
  than a `RuntimeHandle`.

### Breaking Changes

* Use of a `rayon` threadpool to run document actors is now gated behind the
  `threadpool` feature flag and the `RepoBuilder::with_concurrency` method.
* `RuntimeHandle` is now much simpler and only requires a `spawn`function
* `StorageKey` no longer implements `FromIterator<String>` or
  `From<Vec<String>>`, use `StorageKey::from_parts` instead

## 0.3.1 - 2025-08-29

### Fixed

* It was possible for the compaction logic to completely delete a document in
  some cases, fixed in https://github.com/alexjg/samod/pull/19

## 0.3.0 - 2025-08-25

### Added

* Added a `RuntimeHandle` for a `futures::executor::LocalPool`
* Add `Repo::connect_tokio_io` as a convenience for connecting a
  `tokio::io::Async{ReadWrite}` source as a length delimited stream/sink
  combination
* Added a bunch of docs

### Breaking Changes

* Rename `samod::Samod` to `samod::Repo` and `samod::SamodBuilder` to `samod::RepoBuilder`

## 0.2.2 - 2025-08-18

This release is a significant rewrite of the `samod_core` crate to not use
async/await syntax internally. It introduces no changes to `samod` but there
are breaking changes in `samod_core`:

### Breaking Changes to `samod_core`

* `samod_core::ActorResult` is now called `samod_core::DocActorResult` and has
  an additional `stopped` field
* `Hub::load` no longer takes a `rand::Rng` or `UnixTimestamp` argument
* `SamodLoader::step` takes an additional `rand::Rng` argument
* `SamodLoader::provide_io_result` no longer takes a `UnixTimestamp` argument
* `Hub::handle_event` takes an additional `rand::Rng` argument

## 0.2.1 - 2025-08-08

### Fixed

* Fix a deadlock

## 0.2.0 - 2025-08-06

### Fixed

* Make `samod_core::Hub` and `samod_core::SamodLoader` `Send` ([#3](https://github.com/alexjg/samod/pull/3) by @matheus23)
