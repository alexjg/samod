use std::{
    collections::HashMap,
    sync::{Arc, Mutex, mpsc as std_mpsc},
};

use automerge::Automerge;
use conn_handle::ConnHandle;
use futures::{
    Sink, SinkExt, Stream, StreamExt,
    channel::{mpsc, oneshot},
    stream::FuturesUnordered,
};
use rand::SeedableRng;
use samod_core::{
    CommandId, CommandResult, ConnectionId, DocumentActorId, LoaderState, UnixTimestamp,
    actors::{
        DocToHubMsg,
        document::{DocumentActor, SpawnArgs},
        hub::{DispatchedCommand, Hub, HubEvent, HubResults, io::HubIoAction},
    },
    io::{IoResult, IoTask},
    network::{ConnectionEvent, ConnectionState},
};
pub use samod_core::{DocumentId, PeerId, network::ConnDirection};
use tracing::Instrument;

mod actor_task;
use actor_task::ActorTask;
mod actor_handle;
use actor_handle::ActorHandle;
mod announce_policy;
mod builder;
pub use builder::RepoBuilder;
mod conn_finished_reason;
mod conn_handle;
pub use conn_finished_reason::ConnFinishedReason;
mod doc_actor_inner;
mod doc_handle;
mod io_loop;
pub use doc_handle::DocHandle;
mod peer_connection_info;
pub use peer_connection_info::ConnectionInfo;
mod stopped;
pub use stopped::Stopped;
pub mod storage;
use crate::{announce_policy::AlwaysAnnounce, storage::InMemoryStorage};
use crate::{announce_policy::AnnouncePolicy, doc_actor_inner::DocActorInner, storage::Storage};
pub mod runtime;
mod websocket;

#[derive(Clone)]
pub struct Repo {
    inner: Arc<Mutex<Inner>>,
}

impl Repo {
    // Create a new [`Repo`] instance which spawns its tasks onto the provided runtime
    pub fn builder<R: runtime::RuntimeHandle>(
        runtime: R,
    ) -> RepoBuilder<InMemoryStorage, R, AlwaysAnnounce> {
        builder::RepoBuilder::new(runtime)
    }

    // Create a new [`Repo`] instance which spawns it's tasks onto the current tokio runtime
    #[cfg(feature = "tokio")]
    pub fn build_tokio() -> RepoBuilder<InMemoryStorage, ::tokio::runtime::Handle, AlwaysAnnounce> {
        builder::RepoBuilder::new(::tokio::runtime::Handle::current())
    }

    // Create a new [`Repo`] instance which spawns it's tasks onto a [`futures::executor::LocalPool`]
    pub fn build_localpool(
        spawner: futures::executor::LocalSpawner,
    ) -> RepoBuilder<InMemoryStorage, futures::executor::LocalSpawner, AlwaysAnnounce> {
        builder::RepoBuilder::new(spawner)
    }

    // Create a new [`Repo`] instance which spawns its tasks onto the current gio mainloop
    //
    // # Panics
    //
    // This function will panic if called outside of a gio mainloop context.
    #[cfg(feature = "gio")]
    pub fn build_gio()
    -> RepoBuilder<InMemoryStorage, crate::runtime::gio::GioRuntime, AlwaysAnnounce> {
        builder::RepoBuilder::new(crate::runtime::gio::GioRuntime::new())
    }

    // Given a `SamodBuilder`, load the `Samod` instance.
    //
    // This method loads some initial state out of storage before returning a running `Samod` instance
    pub async fn load<R: runtime::RuntimeHandle, S: Storage, A: AnnouncePolicy>(
        builder: RepoBuilder<S, R, A>,
    ) -> Self {
        let RepoBuilder {
            storage,
            runtime,
            peer_id,
            announce_policy,
        } = builder;
        let mut rng = rand::rngs::StdRng::from_rng(&mut rand::rng());
        let peer_id = peer_id.unwrap_or_else(|| PeerId::new_with_rng(&mut rng));
        let mut loading = Hub::load(peer_id.clone());
        let mut running_tasks = FuturesUnordered::new();
        let hub = loop {
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
        };

        let (tx_storage, rx_storage) = mpsc::unbounded();
        let (tx_to_core, rx_from_core) = mpsc::unbounded();
        let inner = Arc::new(Mutex::new(Inner {
            workers: rayon::ThreadPoolBuilder::new().build().unwrap(),
            actors: HashMap::new(),
            hub: *hub,
            pending_commands: HashMap::new(),
            connections: HashMap::new(),
            tx_io: tx_storage,
            tx_to_core,
            waiting_for_connection: HashMap::new(),
            stop_waiters: Vec::new(),
            rng: rand::rngs::StdRng::from_os_rng(),
        }));

        // These futures are spawned on the runtime so they run regardless of awaiting
        #[allow(clippy::let_underscore_future)]
        let _ = runtime.spawn(io_loop::io_loop(
            peer_id.clone(),
            inner.clone(),
            storage,
            announce_policy,
            rx_storage,
        ));
        // These futures are spawned on the runtime so they run regardless of awaiting
        #[allow(clippy::let_underscore_future)]
        let _ = runtime
            .spawn({
                let inner = inner.clone();
                async move {
                    let mut rx = rx_from_core;
                    while let Some((actor_id, msg)) = rx.next().await {
                        let event = HubEvent::actor_message(actor_id, msg);
                        inner.lock().unwrap().handle_event(event);
                    }
                }
            })
            .instrument(tracing::info_span!("actor_loop", local_peer_id=%peer_id));

        Self { inner }
    }

    // Create a new document and return a handle to it
    //
    // # Arguments
    // * `initial_content` - The initial content of the document. If this is an empty document
    //                       a single empty commit will be created and added to the document.
    //                       This ensures that when a document is created, _something_ is in
    //                       storage
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

    // Lookup a document by ID
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

    // Connect a new peer
    //
    // The future returned by this method must be driven to completion in order
    // to continue processing the messages sent by the peer. If the future is
    // dropped, the connection will be closed.
    #[tracing::instrument(skip(self, stream, sink), fields(local_peer_id = tracing::field::Empty))]
    pub fn connect<Str, Snk, SendErr, RecvErr>(
        &self,
        stream: Str,
        mut sink: Snk,
        direction: ConnDirection,
    ) -> impl Future<Output = ConnFinishedReason> + 'static
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
        let DispatchedCommand { command_id, event } = HubEvent::create_connection(direction);
        let (tx, rx) = oneshot::channel();
        self.inner
            .lock()
            .unwrap()
            .pending_commands
            .insert(command_id, tx);
        self.inner.lock().unwrap().handle_event(event);

        let inner = self.inner.clone();
        async move {
            let connection_id = match rx.await {
                Ok(CommandResult::CreateConnection { connection_id }) => connection_id,
                Ok(other) => panic!("unexpected command result for create connection: {other:?}"),
                Err(_) => return ConnFinishedReason::Shutdown,
            };

            let mut rx = {
                let mut rx = inner
                    .lock()
                    .unwrap()
                    .connections
                    .get_mut(&connection_id)
                    .map(|ConnHandle { rx, .. }| (rx.take()))
                    .expect("connection not found");
                rx.take().expect("receive end not found")
            };

            let mut stream = stream.fuse();
            let result = loop {
                futures::select! {
                    next_inbound_msg = stream.next() => {
                        if let Some(msg) = next_inbound_msg {
                            match msg {
                                Ok(msg) => {
                                    let DispatchedCommand { event, .. } = HubEvent::receive(connection_id, msg);
                                    inner.lock().unwrap().handle_event(event);
                                }
                                Err(e) => {
                                    tracing::error!(err=?e, "error receiving, closing connection");
                                    break ConnFinishedReason::ErrorReceiving(e.to_string());
                                }
                            }
                        } else {
                            tracing::debug!("stream closed, closing connection");
                            break ConnFinishedReason::TheyDisconnected;
                        }
                    },
                    next_outbound = rx.next() => {
                        if let Some(next_outbound) = next_outbound {
                            if let Err(e) = sink.send(next_outbound).await {
                                tracing::error!(err=?e, "error sending, closing connection");
                                break ConnFinishedReason::ErrorSending(e.to_string());
                            }
                        } else {
                            tracing::debug!(?connection_id, "connection closing");
                            break ConnFinishedReason::WeDisconnected;
                        }
                    }
                }
            };
            if !(result == ConnFinishedReason::WeDisconnected) {
                let event = HubEvent::connection_lost(connection_id);
                inner.lock().unwrap().handle_event(event);
            }
            if let Err(e) = sink.close().await {
                tracing::error!(err=?e, "error closing sink");
            }
            result
        }
    }

    // Wait for some conncetion to be established with the given remote peer ID
    pub async fn when_connected(&self, peer_id: PeerId) -> Result<(), Stopped> {
        let (tx, rx) = oneshot::channel();
        {
            let mut inner = self.inner.lock().unwrap();

            for info in inner.hub.connections() {
                if let ConnectionState::Connected { their_peer_id, .. } = info.state
                    && their_peer_id == peer_id
                {
                    return Ok(());
                }
            }
            inner
                .waiting_for_connection
                .entry(peer_id)
                .or_default()
                .push(tx);
        }
        match rx.await {
            Ok(()) => Ok(()),
            Err(_) => Err(Stopped), // Stopped
        }
    }

    // The peer ID of this instance
    pub fn peer_id(&self) -> PeerId {
        self.inner.lock().unwrap().hub.peer_id().clone()
    }

    // Stop the `Samod` instance.
    //
    // This will wait until all storage tasks have completed before stopping all
    // the documents and returning
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
    workers: rayon::ThreadPool,
    actors: HashMap<DocumentActorId, ActorHandle>,
    hub: Hub,
    pending_commands: HashMap<CommandId, oneshot::Sender<CommandResult>>,
    connections: HashMap<ConnectionId, ConnHandle>,
    tx_io: mpsc::UnboundedSender<io_loop::IoLoopTask>,
    tx_to_core: mpsc::UnboundedSender<(DocumentActorId, DocToHubMsg)>,
    waiting_for_connection: HashMap<PeerId, Vec<oneshot::Sender<()>>>,
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
                    let (tx, rx) = mpsc::unbounded();
                    self.connections
                        .insert(*connection_id, ConnHandle { tx, rx: Some(rx) });
                }
                let _ = tx.send(command);
            } else {
                tracing::warn!("Received result for unknown command: {:?}", command_id);
            }
        }

        for task in new_tasks {
            match task.action {
                HubIoAction::Send { connection_id, msg } => {
                    if let Some(ConnHandle { tx, .. }) = self.connections.get(&connection_id) {
                        let _ = tx.unbounded_send(msg);
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
                let _ = tx.send(ActorTask::HandleMessage(actor_msg));
            } else {
                tracing::warn!(?actor_id, "received message for unknown actor");
            }
        }

        for evt in connection_events {
            match evt {
                ConnectionEvent::HandshakeCompleted {
                    connection_id: _,
                    peer_info,
                } => {
                    if let Some(tx) = self.waiting_for_connection.get_mut(&peer_info.peer_id) {
                        for tx in tx.drain(..) {
                            let _ = tx.send(());
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
                    // code in Samod::connect to finish
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
        let (tx, rx) = std_mpsc::channel();
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

        let span = tracing::Span::current();
        self.workers.spawn(move || {
            let _enter = span.enter();
            doc_inner.lock().unwrap().handle_results(init_results);

            while let Ok(actor_task) = rx.recv() {
                let mut inner = doc_inner.lock().unwrap();
                inner.handle_task(actor_task);
                if inner.is_stopped() {
                    tracing::debug!(?doc_id, ?actor_id, "actor stopped");
                    break;
                }
            }
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
