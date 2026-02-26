# Changelog

## Unreleased

The main focus of this release is a new connection management API which
replaces the `Repo::connect` method with separate APIs for making outgoing
connections (referred to as a "dialer") and accepting incoming connections
(an "acceptor"). The payoff is that we can automatically handle reconnection.

What follows is a quick guide to the new API; for more details on breaking
changes and added features, see the "Added" and "Breaking Changes" sections
which follow the guide.

### Updating to the New Connection API

#### Outgoing connections (previously `Repo::connect(..., ConnDirection::Outgoing)`)

```rust
// Before
let conn = repo.connect(rx, tx, ConnDirection::Outgoing).unwrap();
conn.handshake_complete().await.unwrap();

// After
let handle = repo.dial(BackoffConfig::default(), Arc::new(my_dialer)).unwrap();

// or, for WebSocket URLs directly:
let handle = repo.dial_websocket(url, BackoffConfig::default()).unwrap();
```

#### Incoming connections (previously `Repo::connect(..., ConnDirection::Incoming)`)

```rust
// Before
let conn = repo.connect(rx, tx, ConnDirection::Incoming).unwrap();

// After — set up an acceptor once, then hand transports to it as they arrive:
let acceptor = repo.make_acceptor(url).unwrap();

// From arbitrary streams/sinks:
acceptor.accept(Transport::new(rx_stream, tx_sink)).unwrap();

// From an axum WebSocket upgrade handler:
acceptor.accept_axum(socket).unwrap();

// From a raw tungstenite stream:
acceptor.accept_tungstenite(ws_stream).unwrap();
```

#### Observing connection events

```rust
// Dialer side — wait for the first successful connection:
let peer_info = handle.established().await?;

// Dialer side — stream every lifecycle event:
let mut events = handle.events();
while let Some(event) = events.next().await {
    match event {
        DialerEvent::Connected { peer_info } => { /* … */ }
        DialerEvent::Disconnected             => { /* … */ }
        DialerEvent::Reconnecting { attempt } => { /* … */ }
        DialerEvent::MaxRetriesReached        => { break; }
    }
}

// Acceptor side — react to clients connecting/disconnecting:
let mut events = acceptor.events();
while let Some(event) = events.next().await {
    match event {
        AcceptorEvent::ClientConnected    { connection_id, peer_info } => { /* … */ }
        AcceptorEvent::ClientDisconnected { connection_id }            => { /* … */ }
    }
}
```

#### Implementing a custom `Dialer`

```rust
use samod::{Dialer, Transport};
use url::Url;
use std::pin::Pin;

struct MyDialer { url: Url }

impl Dialer for MyDialer {
    fn url(&self) -> Url { self.url.clone() }

    fn connect(&self) -> Pin<Box<dyn Future<Output = Result<Transport, Box<dyn std::error::Error + Send + Sync>>> + Send>> {
        Box::pin(async move {
            // establish your transport here, then wrap it:
            Ok(Transport::new(my_rx_stream, my_tx_sink))
        })
    }
}
```

#### Implementing `RuntimeHandle::sleep`

If you have a custom `RuntimeHandle`, add the new required method:

```rust
fn sleep(&self, duration: Duration) -> impl Future<Output = ()> + Send {
    tokio::time::sleep(duration) // or your runtime's equivalent
}
```

### Added

* `Repo::dial` — initiates an outgoing connection using a user-supplied
  `Dialer` implementation. Returns a `DialerHandle` immediately (non-blocking).
  The `DialerHandle` can be used to monitor the connection.
* `Repo::dial_websocket` — convenience wrapper around `Repo::dial` for
  WebSocket connections; takes a `Url` and `BackoffConfig`.
* `Repo::acceptor` — registers a URL as a listening address and returns an
  `AcceptorHandle` for accepting incoming connections on that URL.
* `Transport` — a type-erased `(Stream, Sink)` pair returned by `Dialer::connect`
  and accepted by `AcceptorHandle::accept`.
* `RuntimeHandle::sleep` — required new method on the `RuntimeHandle` trait;
  must return a future that completes after the given `Duration`. This powers
  back-off delays inside the reconnection logic.
* `samod::NeverAnnounce`, an `AnnouncePolicy` which never announces any documents
  to peers
* Add the `native-tls` feature to `tungstenite` and `tokio-tungstenite` when the
  `tungstenite` feature is enabled. This allows using TLS with WebSocket 
  dialers.
* The `samod::tokio_io` module which contains `TcpDialer` for connecting to 
  servers over TCP and `AcceptorHandle::accetp_tokio_io` which implements
  the receiving end

### Fixed

* A bug where requests which were forwarded across peers who were configured
  to not announce documents would fail to resolve on the original requestor

### Breaking Changes

* **`Repo::connect` / `Connection` removed.** The old unified `connect` method
  and the `Connection` / `ConnDirection` types have been removed entirely.
  Use `Repo::dial` (outgoing) and `Repo::acceptor` + `AcceptorHandle::accept`
  (incoming) instead.
* **`RuntimeHandle::sleep` is now required.** Any custom `RuntimeHandle`
  implementation must add a `sleep(duration: Duration) -> impl Future` method.
* **`Repo::connect_tungstenite` / `Repo::accept_axum` / `Repo::accept_tungstenite`
  moved to `AcceptorHandle`.** Call `acceptor.accept_axum(socket)` /
  `acceptor.accept_tungstenite(ws)` instead of the repo directly.
* **`Repo::when_connected` removed.** Replace calls like
  `repo.when_connected(peer_id).await` with `dialer_handle.established().await`
  (dialer side) or listening on `acceptor.events()` for
  `AcceptorEvent::ClientConnected` (acceptor side). Connection IDs are now
  obtained from those futures/events rather than from `when_connected`.
* **`load_local` replaces `load` on `LocalPool` repos.** `RepoBuilder::load_local()`
  must be used instead of `RepoBuilder::load()` when building a repo for a
  `futures::executor::LocalPool` runtime.


## samod-core: 0.7.1 - 2026-01-13

### Added

* Added `DocumentActor::begin_modification` method for modification patterns
  which can't be expressed using a callback

## 0.7.1 - 2026-01-08

### Fixed

* There was a bug where document larger than 8Mb wouldn't sync due to a size
  limit in the length delimited codec used in `Repo::connect_tokio_io`. This
  has been fixed by increasing the limit

## 0.7.0 - 2026-01-07

### Breaking Changes

* `Repo::connected_peers` now returns a stream of changes to the connected
  peers, as well as the currently connected peers. This makes it possible to
  track changes to connected peers consistently

### Fixed

* Fixed a deadlock when using ConcurrencyConfig::Threadpool and loading more
  documents than threads in the pool

## 0.6.1 - 2025-12-08

### Added

* impl `PartialEq` and `Eq` for `AutomergeUrl`

### Fixed

* samod: Use doc_cfg instead of doc_auto_cfg to make docs.rs build work

## 0.6.0 - 2025-12-05

This release is a reasonably chunky change to the API of `samod`. Instead
of having `Repo::connect` return a future which needs to be driven, `Repo::connect`
now handles the connection on the runtime it was created with and returns a
`Connection` object, which can be used to examine the connection.

### Added

* `Repo::connect` is now synchronous and returns a `Connection` object
* `DocHandle::{we_have_their_changes, they_have_our_changes}` methods for 
  waiting for sync to complete
* `DocHandle::peers` for listening to changes to the state of peers connected
  to a given document
* `AutomergeUrl::document_id`

### Breaking Changes

* `Repo::connect` is now synchronous and returns a `Connection` object. This 
  means that where previously you would spawn a future here, you can now just
  call the method and forget about the connection (unless you want it for
  some reason)

## 0.5.1 - 2025-12-02

### Added 

* `From<u32>` and `Into<u32>` impls for  `samod_core::{CommandId, IoTaskId,
  ConnectionId, DocumentActorId}`
* `Clone` implementations for various `samod_core` types
* `DocumentId::as_bytes`
* `UnixTimestamp::from_millis`

### Updated

* Updated to `automerge` 0.7.1

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
