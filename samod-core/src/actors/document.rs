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

pub mod actor;
mod actor_io_access;
use actor_io_access::ActorIoAccess;
mod document_actor_id;
mod document_status;
pub(crate) use document_status::DocumentStatus;
mod compaction;
pub mod errors;
pub mod io;
mod peer_doc_connection;
mod ready;
mod request;
mod spawn_args;

// Internal modules for async runtime
mod actor_state;
use actor_state::ActorState;
mod run;

pub use actor::{ActorResult, DocumentActor, WithDocResult};
pub use document_actor_id::DocumentActorId;
pub use errors::DocumentError;
pub use spawn_args::SpawnArgs;
