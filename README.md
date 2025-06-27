# Samod

> samod: (Old English) simultaneously, at the same time, together

`samod` is an experimental implementation of [`automerge-repo`](https://github.com/automerge/automerge-repo) in Rust. The goals of the project are:

* Interoperability with the JS implementation of `automerge-repo`, both over the network and on disk
* Ergonomic APIs for Rust
* A core implementation which can be used via FFI in other languages
* Predictable performance under load

The project currently consists of two crates:

* `samod-core` - a sans-IO implementation of automerge-repo, intended to be wrapped in a runtime
* `samod` - a higher level rust async/await API which can be used in any async runtime which implements `RuntimeHandle`

# Status

Right now this is very much a work in progress. Most importantly there is no documentation, but there are also probably lots of things which are broken. Feel free to have fun, but don't use this anywhere serious yet, and expect to figure out lots of things on your own.
