use std::collections::HashMap;

use crate::{
    DocumentActorId, DocumentId,
    actors::hub::{CommandId, CommandResult},
};

pub(super) struct PendingCommands {
    pending_find_commands: HashMap<DocumentId, Vec<CommandId>>,
    pending_create_commands: HashMap<DocumentActorId, Vec<CommandId>>,
    completed_commands: Vec<(CommandId, CommandResult)>,
}

impl PendingCommands {
    pub(super) fn new() -> Self {
        Self {
            pending_find_commands: HashMap::new(),
            pending_create_commands: HashMap::new(),
            completed_commands: Vec::new(),
        }
    }

    pub(super) fn add_pending_find_command(
        &mut self,
        document_id: DocumentId,
        command_id: CommandId,
    ) {
        self.pending_find_commands
            .entry(document_id)
            .or_default()
            .push(command_id);
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

    pub(super) fn resolve_pending_find(
        &mut self,
        document_id: &DocumentId,
        actor_id: DocumentActorId,
        found: bool,
    ) {
        if let Some(command_ids) = self.pending_find_commands.remove(document_id) {
            for command_id in command_ids {
                self.completed_commands
                    .push((command_id, CommandResult::FindDocument { actor_id, found }));
            }
        }
    }

    pub(super) fn has_pending_create(&self, doc_actor_id: DocumentActorId) -> bool {
        self.pending_create_commands.contains_key(&doc_actor_id)
    }

    pub(super) fn pop_completed_commands(&mut self) -> Vec<(CommandId, CommandResult)> {
        std::mem::take(&mut self.completed_commands)
    }
}
