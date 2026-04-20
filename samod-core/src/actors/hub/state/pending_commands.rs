use std::collections::HashMap;

use crate::{
    DocumentActorId, DocumentId,
    actors::hub::{CommandId, CommandResult},
};

pub(super) struct PendingCommands {
    pending_create_commands: HashMap<DocumentActorId, Vec<CommandId>>,
    completed_commands: Vec<(CommandId, CommandResult)>,
}

impl PendingCommands {
    pub(super) fn new() -> Self {
        Self {
            pending_create_commands: HashMap::new(),
            completed_commands: Vec::new(),
        }
    }

    pub(super) fn add_pending_create_command(
        &mut self,
        actor_id: DocumentActorId,
        command_id: CommandId,
    ) {
        self.pending_create_commands
            .entry(actor_id)
            .or_default()
            .push(command_id);
    }

    pub(super) fn resolve_pending_create(
        &mut self,
        actor_id: DocumentActorId,
        document_id: &DocumentId,
    ) {
        if let Some(command_ids) = self.pending_create_commands.remove(&actor_id) {
            for command_id in command_ids {
                self.completed_commands.push((
                    command_id,
                    CommandResult::CreateDocument {
                        actor_id,
                        document_id: document_id.clone(),
                    },
                ));
            }
        }
    }

    pub(super) fn pop_completed_commands(&mut self) -> Vec<(CommandId, CommandResult)> {
        std::mem::take(&mut self.completed_commands)
    }
}
