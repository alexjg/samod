use std::collections::HashMap;

use crate::ephemera::{EphemeralMessage, EphemeralSessionId, OutgoingSessionDetails};

pub(crate) struct EphemeralSession {
    counter: u64,
    session_id: EphemeralSessionId,
    session_counts: HashMap<EphemeralSessionId, u64>,
}

impl EphemeralSession {
    pub(crate) fn new<R: rand::Rng>(rng: &mut R) -> Self {
        let session_id = EphemeralSessionId::new_from_rng(rng);
        Self {
            counter: 0,
            session_id,
            session_counts: HashMap::new(),
        }
    }

    pub(crate) fn next_message_session_details(&mut self) -> OutgoingSessionDetails {
        self.counter += 1;
        OutgoingSessionDetails {
            counter: self.counter,
            session_id: self.session_id.clone(),
        }
    }

    pub(crate) fn receive_message(&mut self, msg: EphemeralMessage) -> Option<EphemeralMessage> {
        let EphemeralMessage {
            sender_id: _,
            session_id,
            count,
            data: _,
        } = &msg;
        // This logic exists to prevent gossiped messages from echoing forever in loopy
        // topologies. The idea is that each peer has a unique session ID which they tag
        // their messages with, then each time they send a message they increment the counter
        // they send with locally. This means that we can ignore messages with smaller counters
        // than the largest one we've seen from that session. This does mean we might miss
        // messages if they are sent out of order, but that's okay, ephemeral messages are
        // not guaranteed to be delivered in order, or at all.
        if let Some(current_count) = self.session_counts.get_mut(session_id) {
            if *current_count < *count {
                *current_count = *count;
            } else {
                return None;
            }
        } else {
            self.session_counts.insert(session_id.clone(), *count);
        }
        Some(msg)
    }
}
