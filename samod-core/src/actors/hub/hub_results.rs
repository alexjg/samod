use std::collections::HashMap;

use crate::{
    DocumentActorId,
    actors::{
        HubToDocMsg,
        document::SpawnArgs,
        hub::{CommandId, CommandResult},
    },
    io::IoTask,
    network::ConnectionEvent,
};

use super::io::HubIoAction;

/// Results returned from processing an event in the `Hub`
///
/// `HubResults` contains the outcomes of processing an event through
/// `Hub::handle_event`. This includes any new IO operations that need
/// to be performed by the caller, as well as any commands that have
/// completed execution.
///
/// ## Usage Pattern
///
/// After calling `handle_event`, applications should:
/// 1. Execute all tasks in `new_tasks`
/// 2. Check `completed_commands` for any commands they were tracking
/// 3. Notify the system of IO completion via `Event::io_complete`
#[derive(Debug, Default)]
pub struct HubResults {
    /// IO tasks that must be executed by the calling application.
    ///
    /// Each task represents either a storage operation (load, store, delete)
    /// or a network operation (send message). The caller must execute these
    /// operations and notify completion via `Event::io_complete`.
    ///
    /// Tasks are identified by their `IoTaskId` which must be included
    /// in the completion notification to match results with requests.
    pub new_tasks: Vec<IoTask<HubIoAction>>,

    /// Commands that have completed execution.
    ///
    /// This map contains command results keyed by their `CommandId`.
    /// Applications can use this to retrieve the results of commands
    /// they initiated using `Event` methods.
    ///
    /// Common command results include:
    /// - `CreateConnection`: Returns the new connection ID
    /// - `DisconnectConnection`: Confirms disconnection
    /// - `Receive`: Confirms message processing
    pub completed_commands: HashMap<CommandId, CommandResult>,

    /// Requests to spawn new document actors.
    ///
    /// The caller should create document actor instances for these requests
    /// and begin managing their lifecycle. Each entry contains the unique
    /// actor ID and the document ID it should manage.
    pub spawn_actors: Vec<SpawnArgs>,

    /// Messages to send to document actors.
    ///
    /// The caller should route these messages to the appropriate document
    /// actor instances. Each entry contains the target actor ID and the
    /// message to deliver.
    pub actor_messages: Vec<(DocumentActorId, HubToDocMsg)>,

    /// Connection events emitted during processing.
    ///
    /// These events indicate changes in connection state, such as successful
    /// handshake completion, handshake failures, or connection disconnections.
    /// Applications can use these events to track network connectivity and
    /// respond to connection state changes.
    ///
    /// Events include:
    /// - `HandshakeCompleted`: Connection successfully established with peer
    /// - `HandshakeFailed`: Handshake failed due to protocol or format errors
    /// - `ConnectionEstablished`: Connection ready for document sync
    /// - `ConnectionFailed`: Connection failed or was disconnected
    pub connection_events: Vec<ConnectionEvent>,

    /// Indicates whether the hub is currently stopped.
    pub stopped: bool,
}
