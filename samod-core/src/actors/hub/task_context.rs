use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};

use crate::{
    ConnectionId, DocumentActorId, DocumentId, PeerId, UnixTimestamp,
    actors::{
        document::SpawnArgs,
        driver::ActorIo,
        messages::{Broadcast, DocMessage, HubToDocMsgPayload},
    },
    ephemera::{EphemeralMessage, OutgoingSessionDetails},
    network::{ConnectionEvent, wire_protocol::WireMessage},
};

mod conn_access;
pub(crate) use conn_access::ConnectionAccess;
mod state_access;
pub(crate) use state_access::StateAccess;
mod io_access;
pub(crate) use io_access::IoAccess;

use super::{Hub, State, run::HubOutput};

/// Provides controlled access to shared resources within the samod-core async runtime.
///
/// `TaskContext` is the primary interface that internal futures use to interact with
/// shared state and request IO operations. It ensures safe access to shared data
/// structures while maintaining the ability to perform asynchronous operations.
///
/// ## Design Philosophy
///
/// The TaskContext addresses several key challenges in the samod-core architecture:
///
/// 1. **Shared State Access**: Multiple futures need access to shared state, but Rust's
///    borrowing rules prevent holding `RefCell` guards across await points. TaskContext
///    provides controlled access through `StateAccess` which ensures borrows are scoped.
///
/// 2. **IO Coordination**: Futures need to request IO operations and wait for their
///    completion. TaskContext provides the `io()` method to create and track IO tasks.
///
/// 3. **Time Access**: All operations need access to the current timestamp for
///    consistency across the system.
///
/// ## Usage Pattern
///
/// TaskContext provides controlled access to shared resources within command handlers.
/// It ensures safe access to shared state and provides methods for requesting IO operations.
///
/// ## Thread Safety
///
/// TaskContext is designed for single-threaded async execution. All futures execute
/// serially within the main event loop, making `RefCell` safe for interior mutability
/// without the overhead of `Mutex`.
#[derive(Clone)]
pub(crate) struct TaskContext<R: rand::Rng + Send + Clone> {
    now: Arc<Mutex<UnixTimestamp>>,
    state: Arc<Mutex<State>>,
    io: ActorIo<Hub>,
    rng: R,
}

impl<R: rand::Rng + Send + Clone> TaskContext<R> {
    /// Creates a new TaskContext with the given shared resources.
    ///
    /// This constructor is typically called by the `Driver` when setting up
    /// the async runtime environment.
    ///
    /// # Arguments
    ///
    /// * `now` - Shared reference to the current timestamp
    /// * `tx_output` - Channel for sending output back to the driver
    /// * `state` - Shared reference to the system state
    pub(crate) fn new(
        rng: R,
        now: Arc<Mutex<UnixTimestamp>>,
        io: ActorIo<Hub>,
        state: Arc<Mutex<State>>,
    ) -> Self {
        Self {
            rng,
            now,
            state,
            io,
        }
    }

    /// Provides controlled access to the shared system state.
    ///
    /// Returns a `StateAccess` guard that ensures `RefCell` borrows are properly
    /// scoped and cannot be held across await points. This prevents runtime
    /// panics from multiple borrows.
    ///
    /// # Example Usage
    ///
    /// ```text
    /// // Safe: borrow is scoped within the block
    /// let has_connection = ctx.state().has_connection_id(&connection_id);
    ///
    /// // Also safe: each call gets a fresh borrow
    /// ctx.state().add_connection_id(new_connection_id);
    /// ```
    pub(crate) fn state(&self) -> StateAccess<'_> {
        StateAccess::new(*self.now.lock().unwrap(), self.io(), &self.state)
    }

    /// Provides access to IO operations.
    ///
    /// Returns an `IoAccess` interface that can be used to create and await
    /// IO operations such as network sends.
    ///
    /// # Example Usage
    ///
    /// ```text
    /// // Load data from storage
    /// let data = ctx.io().load(storage_key).await?;
    ///
    /// // Send network message
    /// ctx.io().send(connection_id, message).await?;
    /// ```
    pub(crate) fn io(&self) -> IoAccess {
        IoAccess::new(self.io.clone())
    }

    /// Returns the current timestamp.
    pub(crate) fn now(&self) -> UnixTimestamp {
        *self.now.lock().unwrap()
    }

    pub(crate) fn rng(&mut self) -> &mut R {
        &mut self.rng
    }

    /// Spawns a new document actor.
    pub(crate) fn spawn_actor(
        &self,
        actor_id: DocumentActorId,
        document_id: DocumentId,
        initial_doc: Option<automerge::Automerge>,
        initial_connections: HashMap<ConnectionId, (PeerId, Option<DocMessage>)>,
    ) {
        self.io.emit_event(HubOutput::SpawnActor {
            args: Box::new(SpawnArgs {
                actor_id,
                local_peer_id: self.state().peer_id(),
                document_id,
                initial_content: initial_doc,
                initial_connections,
            }),
        });
    }

    /// Sends a message to a document actor.
    ///
    /// # Arguments
    ///
    /// * `actor_id` - The identifier of the actor to send to
    /// * `message` - The message to send
    pub(crate) fn send_to_actor(&self, actor_id: DocumentActorId, message: HubToDocMsgPayload) {
        self.io
            .emit_event(HubOutput::SendToActor { actor_id, message });
    }

    /// Emits a connection event.
    ///
    /// This method sends a connection event through the driver output channel,
    /// which will be included in the `HubResults` for the
    /// application to process.
    ///
    /// # Arguments
    ///
    /// * `event` - The connection event to emit
    ///
    /// # Example Usage
    ///
    /// ```text
    /// let event = ConnectionEvent::HandshakeCompleted {
    ///     connection_id,
    ///     peer_info,
    /// };
    /// ctx.emit_connection_event(event);
    /// ```
    pub(crate) fn emit_connection_event(&self, event: ConnectionEvent) {
        self.io.emit_event(HubOutput::ConnectionEvent { event });
    }

    pub(crate) fn notify_doc_actors_of_removed_connection(
        &self,
        connection_id: crate::ConnectionId,
    ) {
        for actor_info in self.state.lock().unwrap().document_actors() {
            self.io.emit_event(HubOutput::SendToActor {
                actor_id: actor_info.actor_id,
                message: HubToDocMsgPayload::ConnectionClosed { connection_id },
            });
        }
    }

    pub(crate) fn emit_disconnect_event(&self, connection_id: crate::ConnectionId, error: String) {
        let event = ConnectionEvent::ConnectionFailed {
            connection_id,
            error,
        };
        self.io.emit_event(HubOutput::ConnectionEvent { event });
    }

    /// Fails a connection and emits a disconnect IoTask.
    ///
    /// This is a convenience method that combines the state update and IoTask emission
    /// for connection failures initiated by samod-core. It performs both:
    /// 1. Updates the connection state to Failed
    /// 2. Emits a Disconnect IoTask for the caller to handle
    /// 3. Emits a ConnectionFailed event
    ///
    /// This should be used when samod-core determines a connection should be terminated
    /// (e.g., due to protocol errors, handshake failures, validation errors).
    ///
    /// # Arguments
    ///
    /// * `connection_id` - The ID of the connection to fail
    /// * `error` - Error message describing why the connection failed
    ///
    /// # Example Usage
    ///
    /// ```text
    /// // When detecting a protocol error during handshake
    /// ctx.fail_connection_with_disconnect(&connection_id, "Protocol version mismatch".to_string()).await;
    /// ```
    pub(crate) async fn fail_connection_with_disconnect(
        &self,
        connection_id: crate::ConnectionId,
        error: String,
    ) {
        let Some(connection) = self.state.lock().unwrap().remove_connection(&connection_id) else {
            tracing::warn!(
                ?connection_id,
                "attempting to fail a connection that does not exist"
            );
            return;
        };
        tracing::debug!(?error, remote_peer_id=?connection.remote_peer_id(), "failing connection");

        self.emit_disconnect_event(connection_id, error);
        self.notify_doc_actors_of_removed_connection(connection_id);
        // Emit disconnect IoTask so caller can clean up the network connection
        self.io().disconnect(connection_id).await;
    }

    pub(crate) fn broadcast(
        &self,
        from_actor: DocumentActorId,
        to_connections: Vec<ConnectionId>,
        msg: Broadcast,
    ) {
        let Some(doc_id) = self
            .state
            .lock()
            .unwrap()
            .find_document_for_actor(&from_actor)
        else {
            tracing::warn!(
                ?from_actor,
                "attempting to broadcast from an actor that does not exist"
            );
            return;
        };
        let OutgoingSessionDetails {
            counter,
            session_id,
        } = self.state.lock().unwrap().next_ephemeral_msg_details();
        let state = self.state.lock().unwrap();

        for conn_id in to_connections {
            let Some(conn) = state.get_connection(&conn_id) else {
                continue;
            };
            let Some(their_peer_id) = conn
                .established_connection()
                .map(|c| c.remote_peer_id().clone())
            else {
                continue;
            };
            let msg = match &msg {
                Broadcast::New { msg } => WireMessage::Ephemeral {
                    sender_id: state.peer_id.clone(),
                    target_id: their_peer_id,
                    count: counter,
                    session_id: session_id.to_string(),
                    document_id: doc_id.clone(),
                    data: msg.clone(),
                },
                Broadcast::Gossip {
                    msg:
                        EphemeralMessage {
                            sender_id,
                            session_id,
                            count,
                            data,
                        },
                } => WireMessage::Ephemeral {
                    sender_id: sender_id.clone(),
                    target_id: their_peer_id,
                    count: *count,
                    session_id: session_id.to_string(),
                    document_id: doc_id.clone(),
                    data: data.clone(),
                },
            };
            self.io().send(conn, msg.encode());
        }
    }
}
