//! # `samod-core`
//!
//! This crate provides a sans-IO implementation of networking and storage
//! protocols for synchronizing [`automerge`] docuemnts which is compatible with
//! the `@automerge/automerge-repo` JavaScript package. `samod-core` is intended
//! to be used via FFI and wrapped in a language-specific runtime which provides
//! a more ergonomic API. See the `samod` crate for an example of this in Rust.
//!
//! ## Overview
//!
//! Automerge documents are in-memory data structures which represent the
//! editing history of a JSON-like document.  `automerge` provides functionality
//! for saving and loading files from storage and for synchronizing over the
//! network, but it does not specify a wire protocol for doing this. `samod-core`
//! provides a wire protocol and storage convention for storing and synchronizing
//! many automerge documents.
//!
//! ## Actors
//!
//! Processing tasks for a document can be compute intensive, which means that
//! we want to be able to take advantage of parallelism where we can.
//! `samod-core` doesn't make any assumptions about the runtime though, so it
//! doesn't start threads or spawn tasks. Instead, `samod-core` provides an
//! actor based model. There are two kinds of actors:
//!
//! * [`Hub`](crate::actors::hub::Hub) - The central actor which manages a set of connected peers
//! * [`DocumentActor`](crate::actors::document::DocumentActor) - An actor which is created for each document
//!
//! In both cases the actors expose an API wherein you pass some kind of event
//! data structure to the actor representing something that happened and you
//! get back a data structure describing "effects" that need to take place -
//! retrieving things from storage or sending things to connected peers for
//! example.
//!
//! The hub actor exposes an API for dispatching commands which manage the state
//! of the running actors, whilst the document actor exposes an API for accessing
//! and modifying the state of the document. Typical workflows then will involve
//! creating a hub actor, then using commands to connect other peers, and create
//! or find documents. Creating and finding a document will give you a [`DocumentActorId`]
//! which can then be used to find the actor corresponding to a particular document
//! and interact with the document.
//!
//! ## Typical Workflow
//!
//! A typical workflow for using this library will involve three stages:
//!
//! * Load the [`Hub`](crate::actors::hub::Hub) actor
//! * Run a control loop which passes events to the
//!   [`Hub`](crate::actors::hub::Hub) actor and handles effects requested by the
//!   hub actor (including spawning document actors)
//! * At some point the hub actor will receive a stop
//!   command and stop all the actors before returning a
//!   [`HubResults`](crate::actors::hub::HubResults) which has
//!   `HubResults::stopped == true` at which point we can exit the loop
//!
//! ### Loading the Hub Actor
//!
//! Before the [`Hub`](crate::actors::hub::Hub) actor can even be up and running
//! it needs to load some things from storage. This is represented by the
//! [`SamodLoader`] type. To load the hub actor you first create a
//! [`SamodLoader`], and then perform the IO tasks it requires until it finishes
//! loading, at which point you have a [`Hub`](crate::actors::hub::Hub) actor.
//!
//! ### The Control Loop
//!
//! This is the main loop of the samod application. It's not quite a single
//! loop because there is actually a loop per actor, we can describe this
//! as one loop which manages the hub actor, and one which manages each
//! document.
//!
//! #### Hub Loop
//!
//! The hub loop waits for new "events" to pass to the hub as a
//! [`HubEvent`](crate::actors::hub::HubEvent). Events here are either
//!
//! a) New commands from the application (create a document, find a document etc.)
//! b) Completed storage IO operations
//! c) Network events (a message was received, a connection was created)
//!
//! The loop looks like this then:
//!
//! * Wait for an event
//! * Create a new [`HubEvent`](crate::actors::hub::HubEvent) corresponding to
//!   the incoming event
//! * If the event is a command, note down the command ID returned from the
//!   [`HubEvent`](crate::actors::hub::HubEvent) constructor
//! * Pass the event to [`Hub::handle_event`](crate::actors::hub::Hub::handle_event)
//! * Examine the returned [`HubResults`](crate::actors::hub::HubResults)
//!   * If any command is completed (in
//!     [`HubResults::completed_commands`](crate::actors::hub::HubResults::completed_commands))
//!     note the result of the command completion to notify the application
//!     of the result
//!   * Dispatch any new storage or network events
//!   * If any document actors need to be spawned, somehow enqueue spawning
//!     of the actor
//!   * Route any messages from the hub to document actors to the inboxes of
//!     the document actors
//!
//! #### Document Actor Loop
//!
//! The document actor loop is a bit different to the hub actor loop because it
//! doesn't need to handle commands. However, there is the additional
//! complication that the application will typically need to interact with the
//! automerge document the actor manages. This means that the document actor
//! does not have a single `handle_event` method. Instead, there are three
//! methods which are used to interact with the actor:
//!
//! * [`DocumentActor::handle_message`](crate::actors::document::DocumentActor::handle_message) which is used to handle a message from
//!   the hub actor
//! * [`DocumentActor::handle_io_complete`](crate::actors::document::DocumentActor::handle_io_complete) which is used to handle completed
//!   storage requests
//! * [`DocumentActor::with_document`](crate::actors::document::DocumentActor::with_document) which is passed a closure that has access
//!   to the document
//!
//! The "loop" for a document actor then often looks a bit different to the hub
//! loop. Typically the document actor is inside a mutex which holds both the
//! actor, and a reference to whatever channel is used to communicate with the
//! hub actor and IO. The ways we interact with the actor are then twofold,
//! firstly in a control loop waiting for messages from the hub and IO, and
//! secondly in some kind of lock on the mutex which the application uses.
//!
//! In the loop that would be something like this:
//!
//! * Wait for message from the hub or completed IO
//! * Lock the mutex
//! * Call either [`DocumentActor::handle_message`](crate::actors::document::DocumentActor::handle_message) or
//!   [`DocumentActor::handle_io_complete`](crate::actors::document::DocumentActor::handle_io_complete)
//! * Check the returned [`DocActorResult`](crate::actors::document::DocActorResult) and dispatch any new IO
//! * Unlock the mutex
//!
//! Whilst the application driven interactions would be something like this:
//!
//! * Lock the mutex
//! * Call [`DocumentActor::with_document`](crate::actors::document::DocumentActor::with_document) with a closure that performs the desired action
//! * Check the returned [`DocActorResult`](crate::actors::document::DocActorResult) and dispatch any new IO
//! * Unlock the mutex
//!
//! ## Commands
//!
//! When you want to perform some kind of action on the hub actor you create a
//! [`HubEvent`](crate::actors::hub::HubEvent) and pass it to
//! [`Hub::handle_event`](crate::actors::hub::Hub::handle_event). Some kinds of
//! event are "commands", these are created via static methods on the
//! [`HubEvent`](crate::actors::hub::HubEvent) and will return a
//! [`DispatchedCommand`](crate::actors::hub::DispatchedCommand) which includes
//! both a [`HubEvent`](crate::actors::hub::HubEvent) to pass to
//! [`Hub::handle_event`](crate::actors::hub::Hub::handle_event) and a
//! [`CommandId`]. At some point in the future the
//! [`HubResults`](crate::actors::hub::HubResults) returned by
//! [`Hub::handle_event`](crate::actors::hub::Hub::handle_event) will contain
//! the result of the given command. One example of this would be
//! [`HubEvent::create_document`](crate::actors::hub::HubEvent::create_document)
//! which returns a command ID which will be marked as completed when the
//! document is created and saved to storage.
//!
//! ## IO
//!
//! There are two kinds of IO which `samod-core` performs - interacting with
//! storage and sending messages over the network. In both cases there is some
//! kind of task which `samod-core` requests completion of, the runtime then
//! performs the task and at a later date informs `samod-core` of the result. To
//! track ongoing tasks we use the [`IoTask`](crate::io::IoTask) struct. This
//! contains a [`IoTaskId`](crate::io::IoTaskId) which is used to identify the
//! ongoing task and some kind of action to perform. When the task is completed
//! the runtime constructs an [`IoResult`](crate::io::IoResult) using the
//! original task ID and passes it to the actor which requested the task.
//!
//! ### Storage
//!
//! One very important part of IO is storage. `samod-core` assumes a very simple
//! storage model. Storage is assumed to be a key-value store with range
//! queries. The keys are lists of strings and the values are bytes. For
//! example, a key might be `["documents", "1234567890abcdef"]` and the value
//! might be the bytes of a document. Keys are represented by the [`StorageKey`]
//! type and the operations which storage must support are represented by the
//! [`StorageTask`](crate::io::StorageTask) type.
//!
//! ### Network Connections
//!
//! Network connections are assumed to be stream oriented and are represented by
//! the [`ConnectionId`] type. You obtain a connection by passing a
//! [`HubEvent::create_connection`](crate::actors::hub::HubEvent::create_connection)
//! to the [`Hub`](crate::actors::hub::Hub) and waiting for the create command
//! to complete, returning a [`ConnectionId`]. Messages can now be passed to the
//! hub using [`HubEvent::receive`](crate::actors::hub::HubEvent::receive) and
//! outbound messages will be found in
//! [`HubResults::new_tasks`](crate::actors::hub::HubResults::new_tasks).
//! Finally, the connection can be closed using
//! [`HubEvent::connection_lost`](crate::actors::hub::HubEvent::connection_lost)
//! or, if it is closed by some other event, the notification of that closure
//! will be found in
//! [`HubResults::connection_events`](crate::actors::hub::HubResults::connection_events).
mod automerge_url;
pub use actors::hub::{CommandId, CommandResult};
pub use automerge_url::AutomergeUrl;
pub mod actors;
mod document_changed;
mod document_id;
mod ephemera;
pub mod network;
pub use network::ConnectionId;
pub use network::{
    BackoffConfig, ConnectionOwner, DialRequest, DialerConfig, DialerEvent, DialerId,
    ListenerConfig, ListenerId,
};
mod peer_id;

pub use actors::document::{CompactionHash, DocumentActorId, SyncDirection, SyncMessageStat};
pub use document_changed::DocumentChanged;
pub use document_id::{BadDocumentId, DocumentId};
pub mod io;
pub use peer_id::{PeerId, PeerIdError};
mod storage_key;
pub use storage_key::{InvalidStorageKey, StorageKey};
mod storage_id;
pub use storage_id::{StorageId, StorageIdError};
mod unix_timestamp;
pub use unix_timestamp::UnixTimestamp;

mod loader;
pub use loader::{LoaderState, SamodLoader};
