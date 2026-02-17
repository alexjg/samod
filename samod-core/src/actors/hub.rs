mod command;

pub(crate) use command::Command;
pub use command::{CommandId, CommandResult};
mod connection;
pub(crate) mod dialer;
mod dispatched_command;
pub use dispatched_command::DispatchedCommand;
mod hub_event;
mod hub_input;
pub use hub_event::HubEvent;
pub(crate) use hub_input::HubInput;
mod hub_event_payload;
pub(crate) use hub_event_payload::HubEventPayload;
mod hub_results;
pub use hub_results::HubResults;
pub mod io;
pub(crate) mod listener;
mod state;
pub(crate) use state::State;

use crate::{ConnectionId, PeerId, SamodLoader, StorageId, UnixTimestamp, network::ConnectionInfo};

use super::RunState;

pub struct Hub {
    state: State,
}

impl Hub {
    pub(crate) fn new(state: State) -> Self {
        Hub { state }
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
    pub fn load(peer_id: PeerId) -> SamodLoader {
        SamodLoader::new(peer_id)
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
    #[tracing::instrument(skip(self, rng), fields(event = %event), level = "trace")]
    pub fn handle_event<R: rand::Rng>(
        &mut self,
        rng: &mut R,
        now: UnixTimestamp,
        event: HubEvent,
    ) -> HubResults {
        let mut results = HubResults::default();
        self.state.handle_event(rng, now, event, &mut results);
        results
    }

    /// Returns the storage ID for this samod instance.
    ///
    /// The storage ID is a UUID that identifies the storage layer this peer is
    /// connected to. Multiple peers may share the same storage ID when they're
    /// connected to the same underlying storage (e.g., browser tabs sharing
    /// IndexedDB, processes sharing filesystem storage).
    pub fn storage_id(&self) -> StorageId {
        self.state.storage_id()
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
        self.state.peer_id().clone()
    }

    /// Returns a list of all connection IDs.
    ///
    /// This includes connections in all states: handshaking, established, and failed.
    ///
    /// # Returns
    ///
    /// A vector of all connection IDs currently managed by this instance.
    pub fn connections(&self) -> Vec<ConnectionInfo> {
        self.state.connections()
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
        self.state.established_peers()
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
        self.state.is_connected_to(peer_id)
    }

    pub fn is_stopped(&self) -> bool {
        self.state.run_state() == RunState::Stopped
    }

    /// Find an existing listener for the given URL.
    ///
    /// Returns the `ListenerId` of the first listener whose URL
    /// matches, or `None` if no such listener exists.
    pub fn find_listener_for_url(&self, url: &url::Url) -> Option<crate::ListenerId> {
        self.state.find_listener_for_url(url)
    }

    /// Returns the current attempt count for a dialer.
    ///
    /// Returns `None` if the dialer doesn't exist.
    pub fn dialer_attempt(&self, dialer_id: crate::DialerId) -> Option<u32> {
        self.state.dialer_attempt(dialer_id)
    }
}
