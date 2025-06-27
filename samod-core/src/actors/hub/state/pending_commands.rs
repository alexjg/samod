use std::collections::HashMap;

use futures::channel::oneshot;

use crate::{
    DocumentActorId, DocumentId,
    actors::hub::{CommandId, CommandResult},
};

pub(super) struct PendingCommands {
    pending_find_commands: HashMap<DocumentId, Vec<CommandId>>,
    pending_create_commands: HashMap<DocumentActorId, Vec<CommandId>>,
    pending_command_completions: HashMap<CommandId, oneshot::Sender<CommandResult>>,
}

impl PendingCommands {
    pub(super) fn new() -> Self {
        Self {
            pending_find_commands: HashMap::new(),
            pending_create_commands: HashMap::new(),
            pending_command_completions: HashMap::new(),
        }
    }

    pub(super) fn add_pending_find_command(
        &mut self,
        document_id: DocumentId,
        command_id: CommandId,
        reply: oneshot::Sender<CommandResult>,
    ) {
        self.pending_find_commands
            .entry(document_id)
            .or_default()
            .push(command_id);
        self.pending_command_completions.insert(command_id, reply);
    }

    pub(super) fn add_pending_create_command(
        &mut self,
        actor_id: DocumentActorId,
        command_id: CommandId,
        reply: oneshot::Sender<CommandResult>,
    ) {
        self.pending_create_commands
            .entry(actor_id)
            .or_default()
            .push(command_id);
        self.pending_command_completions.insert(command_id, reply);
    }

    pub(super) fn resolve_pending_create(
        &mut self,
        actor_id: DocumentActorId,
        document_id: &DocumentId,
    ) {
        if let Some(command_ids) = self.pending_create_commands.remove(&actor_id) {
            for command_id in command_ids {
                if let Some(sender) = self.pending_command_completions.remove(&command_id) {
                    let _ = sender.send(CommandResult::CreateDocument {
                        actor_id,
                        document_id: document_id.clone(),
                    });
                }
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
                if let Some(sender) = self.pending_command_completions.remove(&command_id) {
                    let _ = sender.send(CommandResult::FindDocument { actor_id, found });
                }
            }
        }
    }

    pub(super) fn has_pending_create(&self, doc_actor_id: DocumentActorId) -> bool {
        self.pending_create_commands.contains_key(&doc_actor_id)
    }
}
