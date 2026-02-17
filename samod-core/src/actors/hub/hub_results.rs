use std::collections::HashMap;

use crate::{
    DocumentActorId,
    actors::{
        HubToDocMsg,
        document::SpawnArgs,
        hub::{CommandId, CommandResult, connection::Connection},
        messages::HubToDocMsgPayload,
    },
    io::{IoTask, IoTaskId},
    network::{ConnectionEvent, ConnectionOwner, DialRequest, DialerEvent},
};

use super::io::HubIoAction;

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

    /// Indicates whether the hub is currently stopped.
    pub stopped: bool,
}

impl HubResults {
    pub(crate) fn send(&mut self, conn: &Connection, msg: Vec<u8>) {
        tracing::trace!(conn_id=?conn.id(), remote_peer_id=?conn.remote_peer_id(), num_bytes=msg.len(), "sending message");
        self.emit_io_action(HubIoAction::Send {
            connection_id: conn.id(),
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
        self.new_tasks.push(IoTask {
            task_id: IoTaskId::new(),
            action,
        });
        task_id
    }

    pub(crate) fn emit_dial_request(&mut self, request: DialRequest) {
        self.dial_requests.push(request);
    }

    pub(crate) fn emit_connector_event(&mut self, event: DialerEvent) {
        self.dialer_events.push(event);
    }
}
