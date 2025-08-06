mod command;
use std::sync::{Arc, Mutex};

pub(crate) use command::Command;
pub use command::{CommandId, CommandResult};
mod command_handlers;
mod connection;
mod dispatched_command;
pub use dispatched_command::DispatchedCommand;
mod hub_event;
mod run;
pub use hub_event::HubEvent;
use run::{HubInput, HubOutput};
mod hub_event_payload;
pub(crate) use hub_event_payload::HubEventPayload;
mod hub_results;
pub use hub_results::HubResults;
pub mod io;
mod state;
use io::{HubIoAction, HubIoResult};
pub(crate) use state::State;
mod task_context;

use crate::{
    ConnectionId, PeerId, SamodLoader, StorageId, UnixTimestamp, io::IoTask,
    network::ConnectionInfo,
};

use super::{
    HubToDocMsg, RunState,
    driver::{Actor, Driver, StepResult},
};

pub struct Hub {
    pub(crate) driver: crate::actors::driver::Driver<Hub>,
    state: Arc<Mutex<State>>,
}

impl Actor for Hub {
    type IoTaskAction = HubIoAction;
    type IoResult = HubIoResult;
    type StepResults = HubResults;
    type Output = HubOutput;
    type Input = HubInput;
    type Complete = ();

    fn finish_step(
        outputs: Vec<Self::Output>,
        new_io_tasks: Vec<IoTask<HubIoAction>>,
    ) -> Self::StepResults {
        let mut event_results = HubResults::default();
        for evt in outputs {
            match evt {
                HubOutput::CommandCompleted { command_id, result } => {
                    event_results.completed_commands.insert(command_id, result);
                }
                HubOutput::SpawnActor { args } => {
                    event_results.spawn_actors.push(*args);
                }
                HubOutput::SendToActor { actor_id, message } => {
                    event_results
                        .actor_messages
                        .push((actor_id, HubToDocMsg(message)));
                }
                HubOutput::ConnectionEvent { event } => {
                    event_results.connection_events.push(event);
                }
            }
        }
        event_results.new_tasks.extend(new_io_tasks);

        event_results
    }
}

impl Hub {
    pub(crate) fn new<R: rand::Rng + Clone + Send + Sync + 'static>(
        rng: R,
        now: UnixTimestamp,
        state: Arc<Mutex<State>>,
    ) -> Self {
        let driver = Driver::spawn(now, |args| {
            run::run(rng, args.now, state.clone(), args.rx_input, args.io)
        });
        Hub { driver, state }
    }

    /// Begins loading a samod repository.
    ///
    /// This method returns a `SamodLoader` state machine that handles the
    /// initialization process, including loading or generating the storage ID
    /// and performing any other setup operations.
    ///
    /// # Arguments
    ///
    /// * `now` - The current timestamp for initialization
    ///
    /// # Returns
    ///
    /// A `SamodLoader` that will eventually yield a loaded `Samod` instance.
    pub fn load<R: rand::Rng + Clone + Send + Sync + 'static>(
        rng: R,
        now: UnixTimestamp,
        peer_id: PeerId,
    ) -> SamodLoader<R> {
        SamodLoader::new(rng, peer_id, now)
    }

    /// Processes an event and returns any resulting IO tasks or command completions.
    ///
    /// This is the main interface for interacting with samod-core. Events can be
    /// commands to execute, IO completion notifications, or periodic ticks.
    ///
    /// # Arguments
    ///
    /// * `now` - The current timestamp
    /// * `event` - The event to process
    ///
    /// # Returns
    ///
    /// `EventResults` containing:
    /// - `new_tasks`: IO operations that must be performed by the caller
    /// - `completed_commands`: Commands that have finished execution
    #[tracing::instrument(skip(self), fields(event = %event), level = "trace")]
    pub fn handle_event(&mut self, now: UnixTimestamp, event: HubEvent) -> HubResults {
        if self.state.lock().unwrap().run_state() == RunState::Stopped {
            return HubResults {
                stopped: true,
                ..Default::default()
            };
        }
        match event.payload {
            HubEventPayload::IoComplete(result) => {
                self.driver.handle_io_complete(now, result);
            }
            HubEventPayload::Input(input) => {
                self.driver.handle_input(now, input);
            }
        }
        match self.driver.step(now) {
            StepResult::Suspend(results) => results,
            StepResult::Complete { mut results, .. } => {
                self.state.lock().unwrap().set_run_state(RunState::Stopped);
                tracing::trace!("hub stopped");
                results.stopped = true;
                results
            }
        }
    }

    /// Returns the storage ID for this samod instance.
    ///
    /// The storage ID is a UUID that identifies the storage layer this peer is
    /// connected to. Multiple peers may share the same storage ID when they're
    /// connected to the same underlying storage (e.g., browser tabs sharing
    /// IndexedDB, processes sharing filesystem storage).
    pub fn storage_id(&self) -> StorageId {
        self.state.lock().unwrap().storage_id()
    }

    /// Returns the peer ID for this samod instance.
    ///
    /// The peer ID is a unique identifier for this specific peer instance.
    /// It is generated once at startup and used for all connections.
    ///
    /// # Returns
    ///
    /// The peer ID for this instance.
    pub fn peer_id(&self) -> PeerId {
        self.state.lock().unwrap().peer_id().clone()
    }

    /// Returns a list of all connection IDs.
    ///
    /// This includes connections in all states: handshaking, established, and failed.
    ///
    /// # Returns
    ///
    /// A vector of all connection IDs currently managed by this instance.
    pub fn connections(&self) -> Vec<ConnectionInfo> {
        self.state.lock().unwrap().connections()
    }

    /// Returns a list of all established peer connections.
    ///
    /// This only includes connections that have successfully completed the handshake
    /// and are in the established state.
    ///
    /// # Returns
    ///
    /// A vector of tuples containing (connection_id, peer_id) for each established connection.
    pub fn established_peers(&self) -> Vec<(ConnectionId, PeerId)> {
        self.state.lock().unwrap().established_peers()
    }

    /// Checks if this instance is connected to a specific peer.
    ///
    /// # Arguments
    ///
    /// * `peer_id` - The peer ID to check for
    ///
    /// # Returns
    ///
    /// `true` if there is an established connection to the specified peer, `false` otherwise.
    pub fn is_connected_to(&self, peer_id: &PeerId) -> bool {
        self.state.lock().unwrap().is_connected_to(peer_id)
    }

    pub fn is_stopped(&self) -> bool {
        self.state.lock().unwrap().run_state() == RunState::Stopped
    }
}
