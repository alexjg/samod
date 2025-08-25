[![Documentation](https://docs.rs/samod/badge.svg)](https://docs.rs/samod/)
[![Crates.io](https://img.shields.io/crates/v/samod.svg)](https://crates.io/crates/samod)
[![License: MIT](https://img.shields.io/badge/License-MIT-blue.svg)](LICENSE-MIT)

# Samod

> samod: (Old English) simultaneously, at the same time, together

`samod` is an experimental implementation of [`automerge-repo`](https://github.com/automerge/automerge-repo) in Rust. The goals of the project are:

* Interoperability with the JS implementation of `automerge-repo`, both over the network and on disk
* Ergonomic APIs for Rust
* A core implementation which can be used via FFI in other languages
* Predictable performance under load

The project currently consists of two crates:

* `samod` - a high level rust async/await API which can be used in any async runtime which implements `RuntimeHandle`
* `samod-core` - a sans-IO implementation of automerge-repo, intended to be wrapped in a runtime

# Status

Right now this is very much a work in progress. There are probably lots of things which are broken. Feel free to have fun, but don't use this anywhere serious yet. Longer term, my objective is to replace the current [`automerge-repo-rs`](https://github.com/automerge/automerge-repo-rs) codebase with this one.
