# Changelog

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
