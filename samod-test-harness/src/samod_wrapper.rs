use std::{
    collections::{HashMap, VecDeque},
    time::Duration,
};

use automerge::Automerge;
use rand::SeedableRng;
use samod_core::{
    CommandId, CommandResult, ConnectionId, DocumentActorId, DocumentChanged, DocumentId, PeerId,
    StorageId, StorageKey, UnixTimestamp,
    actors::{
        DocumentActor, DocumentError,
        hub::{
            DispatchedCommand, HubEvent,
            io::{HubIoAction, HubIoResult},
        },
    },
    io::{IoResult, IoTask},
    network::{ConnDirection, ConnectionEvent, ConnectionInfo},
};

use crate::{Storage, doc_actor_runner::DocActorRunner};

use super::running_doc_ids::RunningDocIds;

// A wrapper which manages a hub and document actors, as well as simlated storage and networking state
pub struct SamodWrapper {
    // The hub actor we are testing
    hub: samod_core::actors::hub::Hub,
    // The simulated storage for this instance
    storage: Storage,
    // The inbox for events that this instance will process
    pub(super) inbox: VecDeque<HubEvent>,
    completed_commands: HashMap<CommandId, CommandResult>,
    now: UnixTimestamp,
    pub(super) outbox: HashMap<ConnectionId, VecDeque<Vec<u8>>>,
    // Document actors managed by this wrapper
    document_actors: HashMap<DocumentActorId, DocActorRunner>,
    // Connection events captured during event processing
    connection_events: Vec<ConnectionEvent>,
    announce_policy: Box<dyn Fn(DocumentId, PeerId) -> bool>,
}

impl SamodWrapper {
    pub fn new(nickname: String) -> Self {
        Self::new_with_storage(nickname, Storage::new())
    }

    pub fn new_with_storage(nickname: String, mut storage: Storage) -> Self {
        let peer_id = PeerId::from_string(nickname.clone());
        let mut loader = samod_core::SamodLoader::new(peer_id);
        let now = UnixTimestamp::now();

        let mut rng = rand::rngs::StdRng::from_os_rng();

        // Execute the loading process
        let hub = loop {
            match loader.step(&mut rng, now) {
                samod_core::LoaderState::NeedIo(tasks) => {
                    for task in tasks {
                        let result = storage.handle_task(task.action);
                        loader.provide_io_result(IoResult {
                            task_id: task.task_id,
                            payload: result,
                        });
                    }
                }
                samod_core::LoaderState::Loaded(hub) => break hub,
            }
        };

        SamodWrapper {
            hub: *hub,
            storage,
            inbox: VecDeque::new(),
            completed_commands: HashMap::new(),
            now,
            outbox: HashMap::new(),
            document_actors: HashMap::new(),
            connection_events: Vec::new(),
            announce_policy: Box::new(|_, _| true),
        }
    }

    pub fn create_connection(&mut self) -> samod_core::ConnectionId {
        let DispatchedCommand { command_id, event } =
            HubEvent::create_connection(ConnDirection::Outgoing);
        self.inbox.push_back(event);
        self.handle_events();
        let completed_command = self
            .completed_commands
            .remove(&command_id)
            .expect("The create connection command never completed");
        match completed_command {
            CommandResult::CreateConnection { connection_id, .. } => connection_id,
            _ => {
                panic!("Expected a CreateConnection command result, but got {completed_command:?}")
            }
        }
    }

    pub fn create_incoming_connection(&mut self) -> samod_core::ConnectionId {
        let DispatchedCommand { command_id, event } =
            HubEvent::create_connection(ConnDirection::Incoming);
        self.inbox.push_back(event);
        self.handle_events();
        let completed_command = self
            .completed_commands
            .remove(&command_id)
            .expect("The create incoming connection command never completed");
        match completed_command {
            CommandResult::CreateConnection { connection_id, .. } => connection_id,
            _ => {
                panic!("Expected a CreateConnection command result, but got {completed_command:?}")
            }
        }
    }

    pub fn create_document(&mut self) -> RunningDocIds {
        let DispatchedCommand { command_id, event } = HubEvent::create_document(Automerge::new());
        self.inbox.push_back(event);
        self.handle_events();
        let completed_command = self
            .completed_commands
            .remove(&command_id)
            .expect("The create document command never completed");
        match completed_command {
            CommandResult::CreateDocument {
                document_id,
                actor_id,
            } => RunningDocIds {
                doc_id: document_id,
                actor_id,
            },
            _ => panic!("Expected a CreateDocument command result, but got {completed_command:?}"),
        }
    }

    pub fn start_create_document(&mut self) -> CommandId {
        let DispatchedCommand { command_id, event } = HubEvent::create_document(Automerge::new());
        self.inbox.push_back(event);
        self.handle_events();
        command_id
    }

    pub fn check_create_document_result(&mut self, command_id: CommandId) -> Option<RunningDocIds> {
        if let Some(completed_command) = self.completed_commands.remove(&command_id) {
            match completed_command {
                CommandResult::CreateDocument {
                    document_id,
                    actor_id,
                } => Some(RunningDocIds {
                    doc_id: document_id,
                    actor_id,
                }),
                _ => panic!(
                    "Expected a CreateDocument command result, but got {completed_command:?}"
                ),
            }
        } else {
            None // Command not yet completed
        }
    }

    pub fn start_find_document(&mut self, document_id: &DocumentId) -> CommandId {
        let DispatchedCommand { command_id, event } = HubEvent::find_document(document_id.clone());
        self.inbox.push_back(event);
        self.handle_events();
        command_id
    }

    pub fn check_find_document_result(
        &mut self,
        command_id: CommandId,
    ) -> Option<Option<DocumentActorId>> {
        if let Some(completed_command) = self.completed_commands.remove(&command_id) {
            match completed_command {
                CommandResult::FindDocument { found, actor_id } => {
                    Some(if found { Some(actor_id) } else { None })
                }
                _ => {
                    panic!("Expected a FindDocument command result, but got {completed_command:?}")
                }
            }
        } else {
            None // Command not yet completed
        }
    }

    pub fn handle_events(&mut self) {
        for (actor_id, runner) in &mut self.document_actors {
            runner.handle_events(self.now, &mut self.storage, &self.announce_policy);
            self.inbox.extend(
                runner
                    .take_outbox()
                    .into_iter()
                    .map(|msg| HubEvent::actor_message(*actor_id, msg)),
            );
        }
        while let Some(event) = self.inbox.pop_front() {
            self.now += Duration::from_millis(10);
            let mut rng = rand::rng();
            let results = self.hub.handle_event(&mut rng, self.now, event);
            tracing::trace!(?results, "handled event");

            // Handle completed commands
            for (command_id, command_result) in results.completed_commands {
                if matches!(command_result, CommandResult::Receive { .. }) {
                    // Don't store the result of receive commands as we don't wait for them
                    continue;
                }
                assert!(!self.completed_commands.contains_key(&command_id));
                self.completed_commands.insert(command_id, command_result);
            }

            // Capture connection events
            for event in results.connection_events {
                self.connection_events.push(event);
            }

            // Handle IO tasks
            self.execute_io_tasks(results.new_tasks);

            // Handle document actor spawn requests
            for args in results.spawn_actors {
                assert!(
                    !self
                        .document_actors
                        .values()
                        .any(|a| a.document_id() == args.document_id()),
                    "Document actor for this document already exists"
                );
                let actor_id = args.actor_id();
                let actor = DocActorRunner::new(self.now, args);
                self.document_actors.insert(actor_id, actor);
            }

            // Handle messages to document actors
            for (actor_id, message) in results.actor_messages {
                self.document_actors
                    .get_mut(&actor_id)
                    .expect("message for missing actor")
                    .deliver_message_to_inbox(message);
            }

            let mut stopped = Vec::new();
            for (actor_id, runner) in &mut self.document_actors {
                runner.handle_events(self.now, &mut self.storage, &self.announce_policy);
                self.inbox.extend(
                    runner
                        .take_outbox()
                        .into_iter()
                        .map(|msg| HubEvent::actor_message(*actor_id, msg)),
                );
                if runner.is_stopped() {
                    tracing::info!(?actor_id, "removing stopped actor");
                    stopped.push(*actor_id);
                }
            }
            for actor_id in stopped {
                self.document_actors.remove(&actor_id);
            }
        }
    }

    fn execute_io_tasks(&mut self, new_tasks: Vec<IoTask<HubIoAction>>) {
        for new_task in new_tasks {
            let result = match new_task.action {
                HubIoAction::Send { connection_id, msg } => {
                    self.outbox.entry(connection_id).or_default().push_back(msg);
                    HubIoResult::Send
                }
                HubIoAction::Disconnect { connection_id: _ } => {
                    // TODO: actually implement disconnection
                    HubIoResult::Disconnect
                }
            };
            let task_result = IoResult {
                task_id: new_task.task_id,
                payload: result,
            };
            self.inbox.push_back(HubEvent::io_complete(task_result));
        }
    }

    pub fn storage_id(&self) -> StorageId {
        self.hub.storage_id()
    }

    pub fn storage(&self) -> &HashMap<StorageKey, Vec<u8>> {
        &self.storage.0
    }

    pub fn storage_mut(&mut self) -> &mut HashMap<StorageKey, Vec<u8>> {
        &mut self.storage.0
    }

    pub fn push_event(&mut self, event: HubEvent) {
        self.inbox.push_back(event);
    }

    /// Returns the number of active document actors
    pub fn actor_count(&self) -> usize {
        self.document_actors.len()
    }

    /// Checks if a specific document actor exists
    pub fn has_actor(&self, actor_id: &DocumentActorId) -> bool {
        self.document_actors.contains_key(actor_id)
    }

    pub fn get_actor(&self, actor_id: &DocumentActorId) -> Option<&DocumentActor> {
        self.document_actors
            .get(actor_id)
            .map(|runner| runner.actor())
    }

    pub fn with_document<F, R>(&mut self, doc_id: &DocumentId, f: F) -> Result<R, DocumentError>
    where
        F: FnOnce(&mut Automerge) -> R,
    {
        let actor = self
            .document_actors
            .values_mut()
            .find(|d| d.document_id() == doc_id)
            .expect("no actor for document");
        let result = actor.with_document(self.now, f);
        self.handle_events();
        result
    }

    pub fn with_document_by_actor<F, R>(
        &mut self,
        actor_id: DocumentActorId,
        f: F,
    ) -> Result<R, DocumentError>
    where
        F: FnOnce(&mut Automerge) -> R,
    {
        let actor = self
            .document_actors
            .get_mut(&actor_id)
            .expect("no such actor");
        let result = actor.with_document(self.now, f);
        self.handle_events();
        result
    }

    /// Returns the connection events captured during event processing.
    ///
    /// This includes all connection events emitted by the Samod instance,
    /// such as handshake completions, failures, and connection state changes.
    pub fn connection_events(&self) -> &[ConnectionEvent] {
        &self.connection_events
    }

    /// Clears all captured connection events.
    ///
    /// This is useful for test scenarios where you want to start fresh
    /// monitoring for new events.
    pub fn clear_connection_events(&mut self) {
        self.connection_events.clear();
    }

    /// Returns a list of all connection IDs.
    ///
    /// This delegates to the underlying Samod instance.
    pub fn connections(&self) -> Vec<ConnectionInfo> {
        self.hub.connections()
    }

    /// Returns a list of all established peer connections.
    ///
    /// This delegates to the underlying Samod instance.
    pub fn established_peers(&self) -> Vec<(ConnectionId, samod_core::PeerId)> {
        self.hub.established_peers()
    }

    /// Checks if this instance is connected to a specific peer.
    ///
    /// This delegates to the underlying Samod instance.
    pub fn is_connected_to(&self, peer_id: &samod_core::PeerId) -> bool {
        self.hub.is_connected_to(peer_id)
    }

    /// Returns the peer ID for this samod instance.
    ///
    /// This delegates to the underlying Samod instance.
    pub fn peer_id(&self) -> samod_core::PeerId {
        self.hub.peer_id()
    }

    pub fn set_announce_policy(&mut self, policy: Box<dyn Fn(DocumentId, PeerId) -> bool>) {
        self.announce_policy = policy;
    }

    pub fn broadcast(&mut self, actor_id: DocumentActorId, msg: Vec<u8>) {
        self.document_actors
            .get_mut(&actor_id)
            .unwrap()
            .broadcast(self.now, msg);
        self.handle_events();
    }

    pub fn pop_ephemera(&mut self, actor_id: DocumentActorId) -> Vec<Vec<u8>> {
        self.document_actors
            .get_mut(&actor_id)
            .unwrap()
            .pop_ephemera()
    }

    pub fn pop_doc_changed(&mut self, actor_id: DocumentActorId) -> Vec<DocumentChanged> {
        self.document_actors
            .get_mut(&actor_id)
            .unwrap()
            .pop_doc_changed()
    }

    pub fn stop(&mut self) {
        self.inbox.push_back(HubEvent::stop());
        self.handle_events();
        assert!(self.hub.is_stopped());
    }
}
