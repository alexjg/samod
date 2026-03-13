use std::collections::HashMap;

use crate::{ConnectionId, actors::messages::SyncMessage};

#[derive(Debug)]
pub(crate) struct Loading {
    pending_sync_messages: HashMap<ConnectionId, Vec<SyncMessage>>,
}

impl Loading {
    pub(crate) fn new() -> Self {
        Self {
            pending_sync_messages: HashMap::new(),
        }
    }

    pub(crate) fn pending_msg_count(&self) -> usize {
        self.pending_sync_messages
            .values()
            .map(|v| v.len())
            .sum::<usize>()
    }

    pub(crate) fn receive_sync_message(&mut self, conn_id: ConnectionId, msg: SyncMessage) {
        self.pending_sync_messages
            .entry(conn_id)
            .or_default()
            .push(msg);
    }

    pub(crate) fn take_pending_sync_messages(&mut self) -> HashMap<ConnectionId, Vec<SyncMessage>> {
        std::mem::take(&mut self.pending_sync_messages)
    }
}
