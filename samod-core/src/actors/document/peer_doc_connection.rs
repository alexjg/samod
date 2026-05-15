use std::time::{Duration, Instant};

use automerge::{
    Automerge, AutomergeError,
    sync::{self, SyncDoc},
};

use crate::{ConnectionId, PeerId, UnixTimestamp, network::PeerDocState};

#[derive(Debug)]
pub(super) struct PeerDocConnection {
    pub(super) connection_id: ConnectionId,
    pub(super) peer_id: PeerId,
    pub(super) sync_state: sync::State,
    // Track whether we've ever received a request so that we know whether to
    // relay the document to the requestor if we obtain the docuemnt after the
    // request was made
    pub(super) has_requested: bool,
    // Whether this state has changed since the last pop
    dirty: bool,
    state: PeerDocState,
    // Whether to announce this document to this peer
    announce_policy: AnnouncePolicy,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AnnouncePolicy {
    Unknown,
    Loading,
    Announce,
    DontAnnounce,
}

impl PeerDocConnection {
    pub(super) fn new(peer_id: PeerId, connection_id: ConnectionId) -> Self {
        Self {
            connection_id,
            peer_id,
            sync_state: sync::State::new(),
            has_requested: false,
            state: PeerDocState::empty(),
            dirty: false,
            announce_policy: AnnouncePolicy::Unknown,
        }
    }

    pub(super) fn reset_sync_state(&mut self) {
        self.sync_state = sync::State::new();
    }

    pub(super) fn mark_requested(&mut self) {
        if !self.has_requested {
            self.has_requested = true;
            self.dirty = true; // Mark as dirty since the request status changed
        }
    }

    pub(super) fn has_requested(&self) -> bool {
        self.has_requested
    }

    pub(super) fn receive_sync_message(
        &mut self,
        now: UnixTimestamp,
        doc: &mut Automerge,
        msg: sync::Message,
    ) -> Result<Duration, AutomergeError> {
        let start = Instant::now();
        doc.receive_sync_message(&mut self.sync_state, msg)?;
        let duration = start.elapsed();
        self.dirty = true;
        self.state.last_received = Some(now);
        self.state.last_acked_heads = self.sync_state.their_heads.clone();
        self.state.shared_heads = Some(self.sync_state.shared_heads.clone());
        self.state.their_heads = self.sync_state.their_heads.clone();
        Ok(duration)
    }

    pub(super) fn generate_sync_message(
        &mut self,
        now: UnixTimestamp,
        doc: &Automerge,
    ) -> Option<(sync::Message, Duration)> {
        let start = Instant::now();
        let message = doc.generate_sync_message(&mut self.sync_state);
        let duration = start.elapsed();
        if let Some(msg) = &message {
            self.state.last_sent = Some(now);
            self.state.last_sent_heads = Some(msg.heads.clone());
            self.state.shared_heads = Some(self.sync_state.shared_heads.clone());
            self.dirty = true;
        }
        message.map(|msg| (msg, duration))
    }

    pub(super) fn their_heads(&self) -> Option<Vec<automerge::ChangeHash>> {
        self.sync_state.their_heads.clone()
    }

    pub(super) fn pop(&mut self) -> Option<PeerDocState> {
        if self.dirty {
            self.dirty = false;
            Some(self.state.clone())
        } else {
            None
        }
    }

    pub(super) fn state(&self) -> &PeerDocState {
        &self.state
    }

    pub(super) fn announce_policy(&self) -> AnnouncePolicy {
        self.announce_policy
    }

    pub(super) fn set_announce_policy(&mut self, policy: AnnouncePolicy) {
        self.announce_policy = policy;
    }
}
