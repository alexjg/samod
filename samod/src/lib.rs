#![cfg_attr(docsrs, feature(doc_cfg))]
//! # Samod
//!
//! `samod` is a library for building collaborative applications which work offlne
//! and don't require servers (though servers certainly can be useful). This is
//! achieved by representing data as [`automerge`](https://docs.rs/automerge/latest/automerge/)
//! documents. `samod` is wire compatible with the `automerge-repo` JavaScript library.
//!
//! ## What does all that mean?
//!
//! `samod` helps you manage automerge "documents", which are hierarchical data
//! structures composed of maps, lists, text, and primitive values - a little
//! like JSON. Every change you make to a document is recorded and you can move
//! back and forth through the history of a document - it's a bit like Git for
//! JSON. `samod` takes care of storing changes for you and synchronizing them
//! with connected peers. The interesting part is that given this very detailed
//! history which we never discard, we can merge documents with changes which
//! were made concurrently. This means that we can build applications which
//! allow multiple users to edit the same document without having to have all
//! changes go through a server.
//!
//! ## How it all works
//!
//! The library is structured around a [`Repo`], which talks to a [`Storage`]
//! instance and to which you can connect to other peers using
//! [`Repo::connect`](crate::Repo::connect). Once you have a [`Repo`] you can
//! create documents using [`Repo::create`], or look up existing docuements using
//! [`Repo::find`]. In either case you will get back a [`DocHandle`] which you can
//! use to interact with the document.
//!
//! Typically then, your workflow will look like this:
//!
//! * Initialize a `Repo` at application startup, passing it a [`RuntimeHandle`]
//!   implementation and [`Storage`] implementation
//! * Whenever you have connections available (maybe you are connecting to a
//!   sync server, maybe you are receiving peer-to-peer connections) you call
//!   [`Repo::connect`] to add connections to the repo.
//! * Create `DocHandle`s using `Repo::create` and look up existing documents
//!   using `Repo::find`
//! * Modify documents using `DocHandle::with_document`
//!
//! Let's walk through each of those steps.
//!
//! ### Initializing a [`Repo`]
//!
//! To initialize a [`Repo`] you call [`Repo::builder()`] to obtain a
//! [`RepoBuilder`] which you use to configure the repo before calling [`RepoBuilder::load()`]
//! to actually load the repository. For example:
//!
//! ```rust
//! # #[cfg(feature="tokio")]
//! # tokio_test::block_on(async {
//! let repo = samod::Repo::builder(tokio::runtime::Handle::current())
//!     .with_storage(samod::storage::InMemoryStorage::new())
//!     .load()
//!     .await;
//! })
//! ```
//!
//! The first argument to `builder` is an implementation of [`RuntimeHandle`].
//! Default implementations are provided for `tokio` and `gio` which can be
//! conveniently used via [`Repo::build_tokio`] and [`Repo::build_gio`]
//! respectively. The [`RuntimeHandle`] trait is straightforward to implement if
//! you want to use some other async runtime.
//!
//! By default `samod` uses an in-memory storage implementation. This is great
//! for prototyping but in most cases you do actually want to persist data somewhere.
//! In this case you'll need an implementation of [`Storage`] to pass to
//! [`RepoBuilder::with_storage`]
//!
//! It is possible to use [`Storage`] and [`AnnouncePolicy`] implementations which
//! do not produce `Send` futures. In this case you will also need a runtime which
//! can spawn non-`Send` futures. See the [runtimes](#runtimes) section for more
//! details.
//!
//!
//! ### Connecting to peers
//!
//! Once you have a `Repo` you can connect it to peers using [`Repo::connect`].
//! This method accepts a [`Stream`] and a [`Sink`] of `Vec<u8>` represnting
//! a bidirectional communication channel and returns a [`Connection`] object
//! which can be used to find out more information about the connection as it
//! progresses. Here's an example where we use `futures::channel::mpsc` as the
//! transport. We create two repos and connect them with these channels.
//!
//! ```rust
//! # #[cfg(feature="tokio")]
//! use samod::ConnDirection;
//! use futures::{StreamExt, channel::mpsc};
//! use std::convert::Infallible;
//!
//! tokio_test::block_on(async {
//! let alice = samod::Repo::build_tokio().load().await;
//! let bob = samod::Repo::build_tokio().load().await;
//!
//! // Set up bidirectional channels
//! let (tx_to_bob, rx_from_alice) = mpsc::unbounded();
//! let (tx_to_alice, rx_from_bob) = mpsc::unbounded();
//! // This is just to make the types line up, ignore it
//! let rx_from_alice = rx_from_alice.map(Ok::<_, Infallible>);
//! let rx_from_bob = rx_from_bob.map(Ok::<_, Infallible>);
//!
//! // Run the connection futures
//! let alice_to_bob = alice.connect(rx_from_bob, tx_to_bob, ConnDirection::Outgoing).unwrap();
//! bob.connect(rx_from_alice, tx_to_alice, ConnDirection::Incoming);
//!
//! // Now wait for alice to be connected
//! alice_to_bob.handshake_complete().await;
//! });
//! ```
//!
//! If you are using `tokio` and connecting to something like a TCP socket you
//! can use [`Repo::connect_tokio_io`], which reduces some boilerplate:
//!
//! ```rust,no_run
//! # #[cfg(feature="tokio")]
//! # async fn dosomething() {
//! use samod::ConnDirection;
//!
//! let repo: samod::Repo = todo!();
//! let io = tokio::net::TcpStream::connect("sync.automerge.org").await.unwrap();
//! repo.connect_tokio_io(io, ConnDirection::Outgoing);
//! # }
//! ```
//!
//! If you are connecting to JavaScript sync server using WebSockets you can enable the
//! `tungstenite` feature and use [`Repo::connect_websocket`]. If you are accepting
//! websocket connections in an `axum` server you can use [`Repo::accept_axum`].
//!
//! #### Connections
//!
//! The [`Connection`] returned by [`Repo::connect`] can be used to listen for
//! changes to the connection state. Initially a connection is in a
//! "handshaking" state, where we know nothing about the other end. Once the
//! handshake is complete we know the peer ID and (possibly) storage ID of the
//! other end. This information is often useful if you're specifically
//! interested in waiting for the connection to a particular peer to be
//! established before trying to sync changes with them. To wait for the
//! connection to be established you can await [`Connection::handshake_complete`].
//!
//! It's also often useful to know when a connection has been disconnected, to
//! learn about this you can await [`Connection::finished`].
//!
//! ### Managing Documents
//!
//! Once you have a [`Repo`] you can use it to manage [`DocHandle`]s. A
//! [`DocHandle`] represents an [`automerge`] document which the [`Repo`]
//! is managing. "managing" here means a few things:
//!
//! * Any changes made to the document using [`DocHandle::with_document`]
//!   will be persisted to storage and synchronized with connected peers
//!   (subject to the [`AnnouncePolicy`]).
//! * Any changes received from connected peers will be applied to the
//!   document and made visible to the application. You can listen for
//!   these changes using [`DocHandle::changes`].
//!
//! To create a new document you use [`Repo::create`] which will return
//! once the document has been persisted to storage. To look up an existing
//! document you use [`Repo::find`]. This will first look in storage, then
//! if the document is not found in storage it will request the document
//! from all connected peers (again subject to the [`AnnouncePolicy`]). If
//! any peer has the document the future returned by [`Repo::find`] will
//! resolve once we have synchronized with at least one remote peer which
//! has the document.
//!
//! You can make changes to a document using [`DocHandle::with_document`].
//!
//! ### Announce Policies
//!
//! By default, `samod` will announce all the [`DocHandle`]s it is synchronizing
//! to all connected peers and will also send requests to any connected peers
//! when you call [`Repo::find`]. This is often not what you want. To customize
//! this logic you pass an implementation of [`AnnouncePolicy`] to
//! [`RepoBuilder::with_announce_policy`]. Note that `AnnouncePolicy` is implemented
//! for `Fn(&DocumentId) -> bool` so you can just pass a closure if you want.
//!
//! ```rust
//! # #[cfg(feature="tokio")]
//! # tokio_test::block_on(async{
//! let authorized_peer = samod::PeerId::from("alice");
//! let repo = samod::Repo::build_tokio().with_announce_policy(move |_doc_id, peer_id| {
//!    // Only announce documents to alice
//!    &peer_id == &authorized_peer
//! }).load().await;
//! # });
//! ```
//!
//! ## Runtimes
//!
//! [`RuntimeHandle`] is a trait which is intended to abstract over the various
//! runtimes available in the rust ecosystem. The most common runtime is `tokio`.
//! `tokio` is a work-stealing runtime which means that the futures spawned on it
//! must be [`Send`], so that they can be moved between threads. This means that
//! [`RuntimeHandle::spawn`] requires [`Send`] futures. This in turn means that
//! the futures returned by the [`Storage`] and [`AnnouncePolicy`] traits are
//! also [`Send`] so that they can be spawned onto the [`RuntimeHandle`].
//!
//! In many cases though, you may have a runtime which doesn't require [`Send`]
//! futures and you may have storage and announce policy implementations which
//! cannot produce [`Send`] futures. This would often be the case in single
//! threaded runtimes for example. In these cases you can instead implement
//! [`LocalRuntimeHandle`], which doesn't require [`Send`] futures and then
//! you implement [`LocalStorage`] and [`LocalAnnouncePolicy`] traits for
//! your storage and announce policy implementations. You configure all these
//! things via the [`RepoBuilder`] struct. Once you've configured the storage
//! and announce policy implementations to use local variants you can then
//! create a local [`Repo`] using [`RepoBuilder::load_local`].
//!
//! ## Concurrency
//!
//! Typically `samod` will be managing many documents. One for each [`DocHandle`]
//! you retrieve via [`Repo::create`] or [`Repo::find`] but also one for any
//! sync messages received about a particular document from remote peers (e.g.
//! a sync server would have no [`DocHandle`]s open but would still be running
//! many document processes). By default document tasks will be handled on the
//! async runtime provided to the [`RepoBuilder`] but this can be undesirable.
//! Document operations can be compute intensive and so responsiveness may
//! benefit from running them on a separate thread pool. This is the purpose
//! of the [`RepoBuilder::with_concurrency`] method, which allows you to
//! configure how document operations are processed. If you want to use the
//! threadpool approach you will need to enable the `threadpool` feature.
//!
//! ## Why not just Automerge?
//!
//! `automerge` is a low level library. It provides routines for manipulating
//! documents in memory and an abstract data sync protocol. It does not actually
//! hook this up to any kind of network or storage. Most of the work involved
//! in doing this plumbing is straightforward, but if every application does
//! it themselves, we don't end up with interoperable applications. In particular
//! we don't end up with fungible sync servers. One of the core goals of this
//! library is to allow application authors to be agnostic as to where the
//! user synchronises data by implementing a generic network and storage layer
//! which all applications can use.
//!
//! ## Example
//!
//! Here's a somewhat fully featured example of using `samod` to manage a todo list:
//!
//! ```rust
//! # #[cfg(feature="tokio")]
//! # tokio_test::block_on(async {
//! use automerge::{ReadDoc, transaction::{Transactable as _}};
//! use futures::StreamExt as _;
//!
//! use samod::ConnDirection;
//!
//! use std::convert::Infallible;
//!
//! # let _ = tracing_subscriber::fmt().try_init();
//!
//! let repo = samod::Repo::build_tokio().load().await; // You don't have to use tokio
//!
//! // Create an initial skeleton for our todo list
//! let mut initial_doc = automerge::Automerge::new();
//! initial_doc.transact::<_, _, automerge::AutomergeError>(|tx| {
//!     let todos = tx.put_object(automerge::ROOT, "todos", automerge::ObjType::List)?;
//!     Ok(())
//! }).unwrap();
//!
//! // Now create a `samod::DocHandle` using `Repo::create`
//! let doc_handle_1 = repo.create(initial_doc).await.unwrap();
//!
//! // Now, create second repo, representing some other device
//! let repo2 = samod::Repo::build_tokio().load().await;
//!
//! // Connect the two repos to each other
//! let (tx_to_1, rx_from_2) = futures::channel::mpsc::unbounded();
//! let (tx_to_2, rx_from_1) = futures::channel::mpsc::unbounded();
//! repo2.connect(rx_from_1.map(Ok::<_, Infallible>), tx_to_1, ConnDirection::Outgoing);
//! repo.connect(rx_from_2.map(Ok::<_, Infallible>), tx_to_2, ConnDirection::Incoming);
//!
//! // Wait for the second repo to be connected to the first repo
//! repo2.when_connected(repo.peer_id()).await.unwrap();
//!
//! // Now fetch the document on repo2
//! let doc2 = repo2.find(doc_handle_1.document_id().clone()).await.unwrap().unwrap();
//!
//! // Create a todo list item in doc2
//! doc2.with_document(|doc| {
//!     doc.transact(|tx| {
//!        let todos = tx.get(automerge::ROOT, "todos").unwrap().
//!           expect("todos should exist").1;
//!        tx.insert(todos, 0, "Buy milk")?;
//!       Ok::<_, automerge::AutomergeError>(())
//!   }).unwrap();
//! });
//!
//! // Wait for the change to be received on repo1
//! doc_handle_1.changes().next().await.unwrap();
//!
//! // See the the document handle on repo1 reflects the change
//! doc_handle_1.with_document(|doc| {
//!   let todos = doc.get(automerge::ROOT, "todos").unwrap().
//!      expect("todos should exist").1;
//!   let item = doc.get(todos, 0).unwrap().expect("item should exist").0;
//!   let automerge::Value::Scalar(val) = item else {
//!     panic!("item should be a scalar");
//!   };
//!   let automerge::ScalarValue::Str(s) = val.as_ref() else {
//!        panic!("item should be a string");
//!   };
//!   assert_eq!(s, "Buy milk");
//!   Ok::<_, automerge::AutomergeError>(())
//! }).unwrap();
//! # });
//! ```
//!
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};

use automerge::Automerge;
use futures::{
    FutureExt, Sink, SinkExt, Stream, StreamExt, TryStreamExt, channel::oneshot,
    stream::FuturesUnordered,
};
use rand::SeedableRng;
pub use samod_core::{
    AutomergeUrl, ConnectionId, DocumentId, PeerId, StorageId, network::ConnDirection,
};
use samod_core::{
    CommandId, CommandResult, DocumentActorId, LoaderState, UnixTimestamp,
    actors::{
        DocToHubMsg,
        document::{DocumentActor, SpawnArgs},
        hub::{DispatchedCommand, Hub, HubEvent, HubResults, io::HubIoAction},
    },
    io::{IoResult, IoTask},
    network::ConnectionEvent,
};
use tracing::Instrument;

mod actor_task;
use actor_task::ActorTask;
mod actor_handle;
use actor_handle::ActorHandle;
mod announce_policy;
mod builder;
pub use builder::RepoBuilder;
mod conn_finished_reason;
mod connection;
pub use conn_finished_reason::ConnFinishedReason;
pub use connection::Connection;
mod doc_actor_inner;
mod doc_handle;
mod doc_runner;
mod io_loop;
pub use doc_handle::DocHandle;
mod peer_connection_info;
mod peer_info;
pub use peer_connection_info::{ConnectionInfo, ConnectionState, PeerDocState};
pub use peer_info::PeerInfo;
mod stopped;
pub use stopped::Stopped;
pub mod storage;
pub use crate::announce_policy::{AlwaysAnnounce, AnnouncePolicy, LocalAnnouncePolicy};
pub use crate::builder::ConcurrencyConfig;
use crate::{
    connection::ConnectionHandle,
    doc_actor_inner::DocActorInner,
    doc_runner::{DocRunner, SpawnedActor},
    io_loop::{DriveConnectionTask, IoLoopTask},
    storage::Storage,
    unbounded::{UnboundedReceiver, UnboundedSender},
};
use crate::{
    runtime::{LocalRuntimeHandle, RuntimeHandle},
    storage::{InMemoryStorage, LocalStorage},
};
pub mod runtime;
mod unbounded;
pub mod websocket;

/// The entry point to this library
///
/// A [`Repo`] represents a set of running [`DocHandle`]s, active connections to
/// other peers over which we are synchronizing the active [`DocHandle`]s, and
/// an instance of [`Storage`] where document data is persisted.
///
/// Individual documents require exclusive access to mutate (including receiving
/// sync messages). In order to make this non-blocking the `Repo` spawns a
/// task for each document on an underlying threadpool. All method calls on
/// the `Repo` can consequently be called in asynchronous contexts as they do
/// not block.
///
/// ## Lifecycle
///
/// To obtain a [`Repo`] call [`Repo::builder`] (or the various `Repo::build_*`
/// variants specific to different runtimes) to obtain a [`RepoBuilder`] and
/// then call [`RepoBuilder::load`] to actually load the [`Repo`].
///
/// Once you have a repo you can connect new network connections to it using
/// [`Repo::connect`]. You can create new documents using [`Repo::create`] and
/// lookup existing documents using [`Repo::find`].
///
/// When you are finished with a [`Repo`] you can call [`Repo::stop`] to flush
/// everything to storage before shutting down the application.
#[derive(Clone)]
pub struct Repo {
    inner: Arc<Mutex<Inner>>,
}

impl std::fmt::Debug for Repo {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Repo").finish()
    }
}

impl Repo {
    // Create a new [`RepoBuilder`] which will build a [`Repo`] that spawns its
    // tasks onto the provided runtime
    pub fn builder<R: runtime::RuntimeHandle>(
        runtime: R,
    ) -> RepoBuilder<InMemoryStorage, R, AlwaysAnnounce> {
        builder::RepoBuilder::new(runtime)
    }

    /// Create a new [`RepoBuilder`] which will build a [`Repo`] that spawns it's
    /// tasks onto the current tokio runtime
    ///
    /// ## Panics
    /// If called outside of the dynamic scope of a tokio runtime
    #[cfg(feature = "tokio")]
    pub fn build_tokio() -> RepoBuilder<InMemoryStorage, ::tokio::runtime::Handle, AlwaysAnnounce> {
        builder::RepoBuilder::new(::tokio::runtime::Handle::current())
    }

    // Create a new [`RepoBuilder`] which will build a [`Repo`] that spawns it's
    // tasks onto a [`futures::executor::LocalPool`]
    pub fn build_localpool(
        spawner: futures::executor::LocalSpawner,
    ) -> RepoBuilder<InMemoryStorage, futures::executor::LocalSpawner, AlwaysAnnounce> {
        builder::RepoBuilder::new(spawner)
    }

    /// Create a new [`Repo`] instance which will build a [`Repo`] that spawns
    /// its tasks onto the current gio mainloop
    ///
    /// # Panics
    ///
    /// This function will panic if called outside of a gio mainloop context.
    #[cfg(feature = "gio")]
    pub fn build_gio()
    -> RepoBuilder<InMemoryStorage, crate::runtime::gio::GioRuntime, AlwaysAnnounce> {
        builder::RepoBuilder::new(crate::runtime::gio::GioRuntime::new())
    }

    pub(crate) async fn load<R: runtime::RuntimeHandle, S: Storage, A: AnnouncePolicy>(
        builder: RepoBuilder<S, R, A>,
    ) -> Self {
        let RepoBuilder {
            storage,
            runtime,
            peer_id,
            announce_policy,
            concurrency,
        } = builder;
        let task_setup = TaskSetup::new(storage.clone(), peer_id, concurrency).await;
        let inner = task_setup.inner.clone();
        task_setup.spawn_tasks(runtime, storage, announce_policy);
        Self { inner }
    }

    pub(crate) async fn load_local<
        'a,
        R: runtime::LocalRuntimeHandle + 'a,
        S: LocalStorage + 'a,
        A: LocalAnnouncePolicy + 'a,
    >(
        builder: RepoBuilder<S, R, A>,
    ) -> Self {
        let RepoBuilder {
            storage,
            runtime,
            peer_id,
            announce_policy,
            concurrency,
        } = builder;
        let task_setup = TaskSetup::new(storage.clone(), peer_id, concurrency).await;
        let inner = task_setup.inner.clone();
        task_setup.spawn_tasks_local(runtime, storage, announce_policy);
        Self { inner }
    }

    /// Create a new document and return a handle to it
    ///
    /// # Arguments
    /// * `initial_content` - The initial content of the document. If this is an
    ///   empty document a single empty commit will be created and added to the
    ///   document. This ensures that when a document is created, _something_ is
    ///   in storage
    ///
    /// The returned future will resolve once the document has been persisted to storage
    pub async fn create(&self, initial_content: Automerge) -> Result<DocHandle, Stopped> {
        let (tx, rx) = oneshot::channel();
        {
            let DispatchedCommand { command_id, event } =
                HubEvent::create_document(initial_content);
            let mut inner = self.inner.lock().unwrap();
            inner.handle_event(event);
            inner.pending_commands.insert(command_id, tx);
            drop(inner);
        }
        let inner = self.inner.clone();
        match rx.await {
            Ok(r) => match r {
                CommandResult::CreateDocument {
                    actor_id,
                    document_id: _,
                } => {
                    {
                        let inner = inner.lock().unwrap();
                        // By this point the document should have been spawned

                        Ok(inner
                            .actors
                            .get(&actor_id)
                            .map(|ActorHandle { doc: handle, .. }| handle.clone())
                            .expect("actor should exist"))
                    }
                }
                other => {
                    panic!("unexpected command result for create: {other:?}");
                }
            },
            Err(_) => Err(Stopped),
        }
    }

    /// Lookup a document by ID
    ///
    /// The [`Repo`] will first attempt to load the document from [`Storage`] and
    /// if it is not found the [`Repo`] will then request the document from all
    /// connected peers (subject to the configured [`AnnouncePolicy`]). If any peer
    /// responds with the document the future will resolve once we have
    /// synchronized with at least one remote peer which has the document. Otherwise,
    /// the future will resolve to `Ok(None)` once all peers have responded that
    /// they do not have the document.
    pub fn find(
        &self,
        doc_id: DocumentId,
    ) -> impl Future<Output = Result<Option<DocHandle>, Stopped>> + 'static {
        let mut inner = self.inner.lock().unwrap();
        let DispatchedCommand { command_id, event } = HubEvent::find_document(doc_id);
        let (tx, rx) = oneshot::channel();
        inner.pending_commands.insert(command_id, tx);
        inner.handle_event(event);
        drop(inner);
        let inner = self.inner.clone();
        async move {
            match rx.await {
                Ok(r) => match r {
                    CommandResult::FindDocument { actor_id, found } => {
                        if found {
                            // By this point the document should have been spawned
                            let handle = inner
                                .lock()
                                .unwrap()
                                .actors
                                .get(&actor_id)
                                .map(|ActorHandle { doc: handle, .. }| handle.clone())
                                .expect("actor should exist");
                            Ok(Some(handle))
                        } else {
                            Ok(None)
                        }
                    }
                    other => {
                        panic!("unexpected command result for create: {other:?}");
                    }
                },
                Err(_) => Err(Stopped),
            }
        }
    }

    /// Connect a tokio IO stream to the repo
    ///
    /// This is a convenience wrapper which uses tokio_util's length delimited
    /// codec to frame the io and passes it to [`Repo::connect`]. As with
    /// [`Repo::connect`] the returned future must be driven to completion to
    /// keep the connection alive and the `ConnFinishedReason` returned
    /// indicates why the connection ended.
    ///
    /// ## Example
    ///
    /// ```rust,no_run
    /// # #[cfg(feature="tokio")]
    /// # async fn dosomething() {
    /// use samod::ConnDirection;
    ///
    /// let repo: samod::Repo = todo!();
    /// let io = tokio::net::TcpStream::connect("sync.automerge.org").await.unwrap();
    /// repo.connect_tokio_io(io, ConnDirection::Outgoing).unwrap();
    /// # }
    /// ```
    #[cfg(feature = "tokio")]
    pub fn connect_tokio_io<Io: tokio::io::AsyncRead + tokio::io::AsyncWrite + Send + 'static>(
        &self,
        io: Io,
        direction: ConnDirection,
    ) -> Result<Connection, Stopped> {
        let codec = tokio_util::codec::LengthDelimitedCodec::builder()
            // 8gb instead of 8mb lol
            .max_frame_length(8 * 1024 * 1024 * 1024)
            .new_codec();
        let framed =
            tokio_util::codec::Framed::new(io, codec);
        let (write_half, read_half) = framed.split();
        let write_half = write_half
            .with::<Vec<u8>, _, _, std::io::Error>(|msg| std::future::ready(Ok(msg.into())));
        let read_half = read_half.map(|res| match res {
            Ok(bytes) => Ok(bytes.to_vec()),
            Err(e) => Err(e),
        });
        self.connect(read_half, write_half, direction)
    }

    /// Connect a new peer
    ///
    /// The future returned by this method must be driven to completion in order
    /// to continue processing the messages sent by the peer. If the future is
    /// dropped, the connection will be closed.
    ///
    /// The returned future willl resolve to a [`ConnFinishedReason`] indicating why
    /// the connection ended, this can be used to determine whether to attempt to
    /// reconnect.
    #[tracing::instrument(skip(self, stream, sink), fields(local_peer_id = tracing::field::Empty))]
    pub fn connect<Str, Snk, SendErr, RecvErr>(
        &self,
        stream: Str,
        sink: Snk,
        direction: ConnDirection,
    ) -> Result<Connection, Stopped>
    where
        SendErr: std::error::Error + Send + Sync + 'static,
        RecvErr: std::error::Error + Send + Sync + 'static,
        Snk: Sink<Vec<u8>, Error = SendErr> + Send + 'static + Unpin,
        Str: Stream<Item = Result<Vec<u8>, RecvErr>> + Send + 'static + Unpin,
    {
        tracing::Span::current().record(
            "local_peer_id",
            self.inner.lock().unwrap().hub.peer_id().to_string(),
        );
        let mut inner = self.inner.lock().unwrap();
        let DispatchedCommand { command_id, event } = HubEvent::create_connection(direction);
        let (tx_result, mut rx_result) = oneshot::channel();
        inner.pending_commands.insert(command_id, tx_result);
        inner.handle_event(event);
        let connection_id = match rx_result.try_recv() {
            Ok(None) => panic!("CreateConnection command should complete immediately"),
            Ok(Some(CommandResult::CreateConnection { connection_id })) => connection_id,
            Ok(other) => panic!(
                "unexpected command result {:?} for create connection",
                other
            ),
            Err(_) => return Err(Stopped),
        };

        let stream =
            stream.map_err(|e| Box::new(e) as Box<dyn std::error::Error + Send + Sync + 'static>);
        let sink = sink
            .sink_map_err(|e| Box::new(e) as Box<dyn std::error::Error + Send + Sync + 'static>);

        let conn_handle = inner
            .connections
            .get(&connection_id)
            .expect("connection not found")
            .clone();

        let loop_task = IoLoopTask::DriveConnection(DriveConnectionTask {
            conn_handle: conn_handle.clone(),
            stream: stream.boxed(),
            sink: Box::pin(sink),
        });
        inner.tx_io.unbounded_send(loop_task).map_err(|_| Stopped)?;

        Ok(Connection::new(conn_handle))
    }

    /// Wait for some connection to be established with the given remote peer ID
    ///
    /// This will resolve immediately if the peer is already connected, otherwise
    /// it will resolve when a connection with the given peer ID is established.
    pub fn when_connected(
        &self,
        peer_id: PeerId,
    ) -> impl Future<Output = Result<Connection, Stopped>> + 'static {
        let inner = self.inner.clone();
        async move {
            let rx = {
                let mut inner = inner.lock().unwrap();

                for conn_handle in inner.connections.values() {
                    if conn_handle.info().map(|i| i.peer_id).as_ref() == Some(&peer_id) {
                        return Ok(Connection::new(conn_handle.clone()));
                    }
                }
                let (tx, rx) = oneshot::channel();
                inner
                    .waiting_for_connection
                    .entry(peer_id)
                    .or_default()
                    .push(tx);
                rx
            };
            rx.await.map_err(|_| Stopped)
        }
    }

    /// The peer ID of this instance
    pub fn peer_id(&self) -> PeerId {
        self.inner.lock().unwrap().hub.peer_id().clone()
    }

    pub fn connected_peers(
        &self,
    ) -> (
        Vec<ConnectionInfo>,
        impl Stream<Item = Vec<ConnectionInfo>> + Unpin + 'static,
    ) {
        let (tx_events, rx_events) = unbounded::channel();
        let mut inner = self.inner.lock().unwrap();
        inner.conn_listeners.push(tx_events);
        let now = inner
            .hub
            .connections()
            .clone()
            .into_iter()
            .map(|c| c.into())
            .collect();
        (now, rx_events)
    }

    /// Stop the `Samod` instance.
    ///
    /// This will wait until all storage tasks have completed before stopping all
    /// the documents and returning
    pub fn stop(&self) -> impl Future<Output = ()> + 'static {
        let (tx, rx) = oneshot::channel();
        {
            let mut inner = self.inner.lock().unwrap();
            inner.stop_waiters.push(tx);
            inner.handle_event(HubEvent::stop());
        }
        async move {
            if rx.await.is_err() {
                tracing::warn!("stop signal was dropped");
            }
        }
    }
}

struct Inner {
    doc_runner: DocRunner,
    actors: HashMap<DocumentActorId, ActorHandle>,
    hub: Hub,
    pending_commands: HashMap<CommandId, oneshot::Sender<CommandResult>>,
    connections: HashMap<ConnectionId, ConnectionHandle>,
    conn_listeners: Vec<unbounded::UnboundedSender<Vec<ConnectionInfo>>>,
    tx_io: UnboundedSender<io_loop::IoLoopTask>,
    tx_to_core: UnboundedSender<(DocumentActorId, DocToHubMsg)>,
    waiting_for_connection: HashMap<PeerId, Vec<oneshot::Sender<Connection>>>,
    stop_waiters: Vec<oneshot::Sender<()>>,
    rng: rand::rngs::StdRng,
}

impl Inner {
    #[tracing::instrument(skip(self, event), fields(local_peer_id=%self.hub.peer_id()))]
    fn handle_event(&mut self, event: HubEvent) {
        let now = UnixTimestamp::now();
        let HubResults {
            new_tasks,
            completed_commands,
            spawn_actors,
            actor_messages,
            stopped,
            connection_events,
        } = self.hub.handle_event(&mut self.rng, now, event);

        for spawn_args in spawn_actors {
            self.spawn_actor(spawn_args);
        }

        for (command_id, command) in completed_commands {
            if let CommandResult::Receive { .. } = &command {
                // We don't track receive commands
                continue;
            }
            if let Some(tx) = self.pending_commands.remove(&command_id) {
                if let CommandResult::CreateConnection { connection_id } = &command {
                    self.connections
                        .insert(*connection_id, ConnectionHandle::new(*connection_id));
                }
                let _ = tx.send(command);
            } else {
                tracing::warn!("Received result for unknown command: {:?}", command_id);
            }
        }

        for task in new_tasks {
            match task.action {
                HubIoAction::Send { connection_id, msg } => {
                    if let Some(connhandle) = self.connections.get(&connection_id) {
                        connhandle.send(msg);
                    } else {
                        tracing::warn!(
                            "Tried to send message on unknown connection: {:?}",
                            connection_id
                        );
                    }
                }
                HubIoAction::Disconnect { connection_id } => {
                    if self.connections.remove(&connection_id).is_none() {
                        tracing::warn!(
                            "Tried to disconnect unknown connection: {:?}",
                            connection_id
                        );
                    }
                }
            }
        }

        for (actor_id, actor_msg) in actor_messages {
            if let Some(ActorHandle { tx, .. }) = self.actors.get(&actor_id) {
                let _ = tx.unbounded_send(ActorTask::HandleMessage(actor_msg));
            } else {
                tracing::warn!(?actor_id, "received message for unknown actor");
            }
        }

        if !connection_events.is_empty() && !self.conn_listeners.is_empty() {
            let new_infos = self.hub.connections();
            self.conn_listeners.retain(|tx| {
                tx.unbounded_send(new_infos.clone().into_iter().map(|c| c.into()).collect())
                    .is_ok()
            });
        }

        for evt in connection_events {
            match evt {
                ConnectionEvent::HandshakeCompleted {
                    connection_id,
                    peer_info,
                } => {
                    if let Some(conn_handle) = self.connections.get(&connection_id) {
                        conn_handle.notify_handshake_complete(peer_info.clone().into());
                        if let Some(tx) = self.waiting_for_connection.get_mut(&peer_info.peer_id) {
                            for tx in tx.drain(..) {
                                let _ = tx.send(Connection::new(conn_handle.clone()));
                            }
                        }
                    }
                }
                ConnectionEvent::ConnectionFailed {
                    connection_id,
                    error,
                } => {
                    tracing::error!(
                        ?connection_id,
                        ?error,
                        "connection failed, notifying waiting tasks",
                    );
                    // This will drop the sender which will in turn cause the stream handling
                    // code in io_loop::drive_connection to emit disconnection events
                    self.connections.remove(&connection_id);
                }
                _ => {}
            }
        }

        if stopped {
            for waiter in self.stop_waiters.drain(..) {
                let _ = waiter.send(());
            }
        }
    }

    #[tracing::instrument(skip(self, args))]
    fn spawn_actor(&mut self, args: SpawnArgs) {
        let (tx, rx) = unbounded::channel();
        let actor_id = args.actor_id();
        let doc_id = args.document_id().clone();
        let (actor, init_results) = DocumentActor::new(UnixTimestamp::now(), args);

        let doc_inner = Arc::new(Mutex::new(DocActorInner::new(
            doc_id.clone(),
            actor_id,
            actor,
            self.tx_to_core.clone(),
            self.tx_io.clone(),
        )));
        let handle = DocHandle::new(doc_id.clone(), doc_inner.clone());
        self.actors.insert(
            actor_id,
            ActorHandle {
                inner: doc_inner.clone(),
                tx,
                doc: handle,
            },
        );

        match &mut self.doc_runner {
            #[cfg(feature = "threadpool")]
            DocRunner::Threadpool(threadpool) => {
                let span = tracing::Span::current();
                threadpool.spawn(move || {
                    let _enter = span.enter();
                    doc_inner.lock().unwrap().handle_results(init_results);

                    while let Ok(actor_task) = rx.recv_blocking() {
                        let mut inner = doc_inner.lock().unwrap();
                        inner.handle_task(actor_task);
                        if inner.is_stopped() {
                            tracing::debug!(?doc_id, ?actor_id, "actor stopped");
                            break;
                        }
                    }
                });
            }
            DocRunner::Async { tx } => {
                if tx
                    .unbounded_send(SpawnedActor {
                        doc_id,
                        actor_id,
                        inner: doc_inner,
                        rx_tasks: rx,
                        init_results,
                    })
                    .is_err()
                {
                    tracing::error!(?actor_id, "actor spawner is gone");
                }
            }
        }
    }
}

/// Spawns a task which listens for new actors to spawn and runs them
///
/// `samod` has two ways of running document actors, on a rayon threadpool, or
/// on the async runtime which was provided to the `SamodBuilder`. In the latter
/// case we don't actually hold on to a reference to the `RuntimeHandle` because
/// that requires it to be `Send` which is not always the case (e.g. when using
/// futures::executor::LocalPool). Instead, we spawn a task on the runtime which
/// listens on a channel for new actors to spawn and then runs them on a
/// `FuturesUnordered`. This function is that task.
async fn async_actor_runner(rx: UnboundedReceiver<SpawnedActor>) {
    let mut running_actors = FuturesUnordered::new();

    loop {
        futures::select! {
            spawn_actor = rx.recv().fuse() => {
                match spawn_actor {
                    Err(_e) => {
                        tracing::trace!("actor spawner task finished");
                        break;
                    }
                    Ok(SpawnedActor { inner, rx_tasks, init_results, doc_id, actor_id }) => {
                        running_actors.push(async move {
                            inner.lock().unwrap().handle_results(init_results);

                            while let Ok(actor_task) = rx_tasks.recv().await {
                                let mut inner = inner.lock().unwrap();
                                inner.handle_task(actor_task);
                                if inner.is_stopped() {
                                    tracing::debug!(?doc_id, ?actor_id, "actor stopped");
                                    break;
                                }
                            }
                        });
                    }
                }
            },
            _ = running_actors.next() => {
                // nothing to do
            }
        }
    }

    // Wait for all actors to stop
    while running_actors.next().await.is_some() {
        // nothing to do
    }
}

/// All the information needed to spawn the background tasks
///
/// When we construct a `Repo` we need to spawn a number of tasks onto the
/// runtime to do things like handle storage tasks. We have to split the
/// spawn process into two stages:
///
/// * Create the channels which are used to communicate with the background tasks
/// * Spawn the background tasks onto the runtime
///
/// The reason we have to split into these two stages is so that we can work with
/// runtimes that don't support non-`Send` tasks. This split is represented by the
/// `TaskSetup::spawn_tasks` and `TaskSetup::spawn_tasks_local` methods.
struct TaskSetup {
    peer_id: PeerId,
    inner: Arc<Mutex<Inner>>,
    rx_storage: UnboundedReceiver<IoLoopTask>,
    rx_from_core: UnboundedReceiver<(DocumentActorId, DocToHubMsg)>,
    rx_actor: Option<UnboundedReceiver<SpawnedActor>>,
}

impl TaskSetup {
    async fn new<S: LocalStorage>(
        storage: S,
        peer_id: Option<PeerId>,
        concurrency: ConcurrencyConfig,
    ) -> TaskSetup {
        let mut rng = rand::rngs::StdRng::from_rng(&mut rand::rng());
        let peer_id = peer_id.unwrap_or_else(|| PeerId::new_with_rng(&mut rng));
        let hub = load_hub(storage.clone(), Hub::load(peer_id.clone())).await;

        let (tx_storage, rx_storage) = unbounded::channel();
        let (tx_to_core, rx_from_core) = unbounded::channel();
        let rx_actor: Option<UnboundedReceiver<SpawnedActor>>;
        let doc_runner = match concurrency {
            #[cfg(feature = "threadpool")]
            ConcurrencyConfig::Threadpool(threadpool) => {
                rx_actor = None;
                DocRunner::Threadpool(threadpool)
            }
            ConcurrencyConfig::AsyncRuntime => {
                let (tx, rx) = unbounded::channel();
                rx_actor = Some(rx);
                DocRunner::Async { tx }
            }
        };

        let inner = Arc::new(Mutex::new(Inner {
            doc_runner,
            actors: HashMap::new(),
            hub: *hub,
            pending_commands: HashMap::new(),
            connections: HashMap::new(),
            conn_listeners: Vec::new(),
            tx_io: tx_storage,
            tx_to_core,
            waiting_for_connection: HashMap::new(),
            stop_waiters: Vec::new(),
            rng: rand::rngs::StdRng::from_os_rng(),
        }));

        TaskSetup {
            peer_id,
            inner,
            rx_actor,
            rx_from_core,
            rx_storage,
        }
    }
    fn spawn_tasks_local<R: LocalRuntimeHandle, S: LocalStorage, A: LocalAnnouncePolicy>(
        self,
        runtime: R,
        storage: S,
        announce_policy: A,
    ) {
        runtime.spawn(
            io_loop::io_loop(
                self.peer_id.clone(),
                self.inner.clone(),
                storage,
                announce_policy,
                self.rx_storage,
            )
            .boxed_local(),
        );
        runtime.spawn({
            let peer_id = self.peer_id.clone();
            let inner = self.inner.clone();
            async move {
                let rx = self.rx_from_core;
                while let Ok((actor_id, msg)) = rx.recv().await {
                    let event = HubEvent::actor_message(actor_id, msg);
                    inner.lock().unwrap().handle_event(event);
                }
            }
            .instrument(tracing::info_span!("actor_loop", local_peer_id=%peer_id))
            .boxed_local()
        });
        if let Some(rx_actor) = self.rx_actor {
            runtime.spawn(async_actor_runner(rx_actor).boxed_local());
        }
    }

    fn spawn_tasks<R: RuntimeHandle, S: Storage, A: AnnouncePolicy>(
        self,
        runtime: R,
        storage: S,
        announce_policy: A,
    ) {
        runtime.spawn(
            io_loop::io_loop(
                self.peer_id.clone(),
                self.inner.clone(),
                storage,
                announce_policy,
                self.rx_storage,
            )
            .boxed(),
        );
        runtime.spawn({
            let peer_id = self.peer_id.clone();
            let inner = self.inner.clone();
            async move {
                let rx = self.rx_from_core;
                while let Ok((actor_id, msg)) = rx.recv().await {
                    let event = HubEvent::actor_message(actor_id, msg);
                    inner.lock().unwrap().handle_event(event);
                }
            }
            .instrument(tracing::info_span!("actor_loop", local_peer_id=%peer_id))
            .boxed()
        });
        if let Some(rx_actor) = self.rx_actor {
            runtime.spawn(async_actor_runner(rx_actor).boxed());
        }
    }
}

async fn load_hub<S: LocalStorage>(storage: S, mut loading: samod_core::SamodLoader) -> Box<Hub> {
    let mut rng = rand::rngs::StdRng::from_os_rng();
    let mut running_tasks = FuturesUnordered::new();
    loop {
        match loading.step(&mut rng, UnixTimestamp::now()) {
            LoaderState::NeedIo(items) => {
                for IoTask {
                    task_id,
                    action: task,
                } in items
                {
                    let storage = storage.clone();
                    running_tasks.push(async move {
                        let result = io_loop::dispatch_storage_task(task, storage).await;
                        (task_id, result)
                    })
                }
            }
            LoaderState::Loaded(hub) => break hub,
        }
        let (task_id, next_result) = running_tasks.select_next_some().await;
        loading.provide_io_result(IoResult {
            task_id,
            payload: next_result,
        });
    }
}

#[cfg(test)]
mod tests {
    use std::marker::PhantomData;

    fn assert_send<S: Send>(_s: PhantomData<S>) {}

    #[cfg(feature = "tokio")]
    fn assert_send_value<S: Send>(_s: impl Fn() -> S) {}

    #[test]
    fn make_sure_it_is_send() {
        assert_send::<super::storage::InMemoryStorage>(PhantomData);
        assert_send::<super::Repo>(PhantomData);

        #[cfg(feature = "tokio")]
        assert_send_value(|| crate::Repo::build_tokio().load());
    }
}
