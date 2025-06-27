use crate::actors::hub::CommandId;

use super::HubEvent;

/// A command that has been prepared for execution with a unique identifier.
///
/// `DispatchedCommand` represents a command that has been assigned a unique
/// `CommandId` and wrapped in an `Event` ready for processing. This structure
/// allows callers to track command completion by matching the command ID
/// with results returned from `EventResults::completed_commands`.
///
/// ## Usage Pattern
///
/// 1. Create a command using `Event` static methods (e.g., `Event::create_connection(ConnDirection::Outgoing)`)
/// 2. Store the `command_id` to track completion
/// 3. Pass the `event` to `Samod::handle_event`
/// 4. Check for completion using the `command_id` in subsequent `EventResults`
pub struct DispatchedCommand {
    /// The unique identifier for this command.
    ///
    /// This ID can be used to match command results when they become available
    /// in `EventResults::completed_commands`.
    pub command_id: CommandId,

    /// The event containing the command to be processed.
    ///
    /// This event should be passed to `Samod::handle_event` to execute the command.
    pub event: HubEvent,
}
