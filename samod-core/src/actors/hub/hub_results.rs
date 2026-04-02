use std::collections::HashMap;

use crate::{
    ConnectionId, DocumentActorId,
    actors::{
        HubToDocMsg,
        document::SpawnArgs,
        hub::{CommandId, CommandResult},
        messages::HubToDocMsgPayload,
    },
    io::{IoTask, IoTaskId},
    network::{ConnectionEvent, ConnectionOwner, DialRequest, DialerEvent},
};

use super::io::HubIoAction;

/// Events related to subduction sync, useful for metrics and testing.
#[cfg(feature = "subduction")]
#[derive(Debug, Clone)]
pub enum SubductionEvent {
    /// A batch sync completed, downloading data from a peer.
    BatchSyncComplete {
        document_id: crate::DocumentId,
        /// Number of loose commits downloaded.
        commits_downloaded: usize,
        /// Number of fragments (chunks) downloaded.
        fragments_downloaded: usize,
    },
}

/// Results returned from processing an event in the `Hub`
///
/// `HubResults` contains the outcomes of processing an event through
/// `Hub::handle_event`. This includes any new IO operations that need
/// to be performed by the caller, as well as any commands that have
/// completed execution.
#[derive(Debug, Default, Clone)]
pub struct HubResults {
    /// IO tasks that must be executed by the calling application.
    pub new_tasks: Vec<IoTask<HubIoAction>>,

    /// Commands that have completed execution.
    pub completed_commands: HashMap<CommandId, CommandResult>,

    /// Requests to spawn new document actors.
    pub spawn_actors: Vec<SpawnArgs>,

    /// Messages to send to document actors.
    pub actor_messages: Vec<(DocumentActorId, HubToDocMsg)>,

    /// Connection events emitted during processing.
    pub connection_events: Vec<ConnectionEvent>,

    /// Requests for the IO layer to establish new transports for dialers.
    ///
    /// When a dialer needs a connection (either on initial registration
    /// or after a reconnection backoff expires), the hub emits a `DialRequest`.
    /// The IO layer should attempt to create the transport and report success via
    /// `HubEvent::create_dialer_connection` or failure via `HubEvent::dial_failed`.
    pub dial_requests: Vec<DialRequest>,

    /// Dialer lifecycle events (e.g. max retries reached).
    pub dialer_events: Vec<DialerEvent>,

    /// Subduction sync events (metrics, status changes).
    #[cfg(feature = "subduction")]
    pub subduction_events: Vec<SubductionEvent>,

    /// Indicates whether the hub is currently stopped.
    pub stopped: bool,

    /// Diagnostics: name of the event type that was processed.
    pub event_type: &'static str,

    /// Diagnostics: number of active connections after processing.
    pub connections_count: usize,

    /// Diagnostics: number of active document actors after processing.
    pub documents_count: usize,
}

impl HubResults {
    pub(crate) fn send(&mut self, conn_id: ConnectionId, msg: Vec<u8>) {
        self.emit_io_action(HubIoAction::Send {
            connection_id: conn_id,
            msg,
        });
    }

    pub(crate) fn emit_disconnect_event(
        &mut self,
        connection_id: crate::ConnectionId,
        owner: ConnectionOwner,
        error: String,
    ) {
        let event = ConnectionEvent::ConnectionFailed {
            connection_id,
            owner,
            error,
        };
        self.connection_events.push(event);
    }

    pub(crate) fn emit_connection_event(&mut self, event: ConnectionEvent) {
        self.connection_events.push(event);
    }

    pub(crate) fn send_to_doc_actor(&mut self, actor_id: DocumentActorId, msg: HubToDocMsgPayload) {
        self.actor_messages.push((actor_id, HubToDocMsg(msg)));
    }

    pub(crate) fn emit_spawn_actor(&mut self, args: SpawnArgs) {
        self.spawn_actors.push(args)
    }

    pub(crate) fn emit_io_action(&mut self, action: HubIoAction) -> IoTaskId {
        let task_id = IoTaskId::new();
        self.new_tasks.push(IoTask { task_id, action });
        task_id
    }

    pub(crate) fn emit_dial_request(&mut self, request: DialRequest) {
        self.dial_requests.push(request);
    }

    pub(crate) fn emit_connector_event(&mut self, event: DialerEvent) {
        self.dialer_events.push(event);
    }
}
