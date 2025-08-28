//! Document actor implementation for managing automerge documents.
//!
//! Document actors are passive state machines that manage individual documents.
//! They handle loading documents from storage, saving them when needed, and
//! managing their lifecycle.
//!
//! ## Architecture
//!
//! - **State machines**: Actors process messages and return results
//! - **Sans-IO**: All I/O operations are returned as tasks for the caller to execute
//! - **Simple lifecycle**: Initialize → Load → Ready → Terminate
//!
//! ## Usage
//!
//! ```text
//! // Create an actor
//! let actor = DocumentActor::new(document_id);
//!
//! // Initialize it
//! let result = actor.handle_message(now, SamodToActorMessage::Initialize)?;
//!
//! // Execute I/O tasks
//! for io_task in result.io_tasks {
//!     let io_result = execute_io(io_task)?;
//!     actor.handle_io_complete(now, io_result)?;
//! }
//! ```

mod doc_actor_result;
pub mod document_actor;
pub use doc_actor_result::DocActorResult;
mod document_actor_id;
mod document_status;
pub(crate) use document_status::DocumentStatus;
pub mod errors;
pub mod io;
mod load;
mod on_disk_state;
pub use on_disk_state::CompactionHash;
mod peer_doc_connection;
mod ready;
mod request;
mod spawn_args;
mod with_doc_result;
pub use with_doc_result::WithDocResult;

// Internal modules for async runtime
mod actor_input;
mod doc_state;
pub(crate) use actor_input::ActorInput;

pub use document_actor::DocumentActor;
pub use document_actor_id::DocumentActorId;
pub use errors::DocumentError;
pub use spawn_args::SpawnArgs;
