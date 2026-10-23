#[cfg(test)]
mod tests;

use std::time::{Duration, Instant};

use automerge::{
    Automerge, AutomergeError, ChangeHash,
    sync::{self, SyncDoc},
};

use crate::actors::messages::SyncMessage;
use crate::{ConnectionId, PeerId, ShareConfig, UnixTimestamp, network::PeerDocState};

#[derive(Debug)]
pub(crate) struct PeerDocConnection {
    pub(super) connection_id: ConnectionId,
    pub(super) peer_id: PeerId,
    pub(super) sync_state: sync::State,
    // Remember inbound requests or sync attempts, even while denied/paused,
    // so that granting access can answer them without proactive announcement.
    pub(super) has_requested: bool,
    // Whether this state has changed since the last pop
    dirty: bool,
    sync_due: bool,
    heads_at_last_generate: Option<Vec<automerge::ChangeHash>>,
    #[cfg(test)]
    generate_calls: usize,
    state: PeerDocState,
    pub(super) share_policy: SharePolicyState,
    last_share_config: Option<ShareConfig>,
    // Preserve message ordering while the asynchronous policy lookup is pending.
    pub(super) pending_sync_messages: Vec<(SyncMessage, UnixTimestamp)>,
    peer_heads: Option<Vec<ChangeHash>>,
    // Where the sync state last started over after a message failed: our
    // heads and the peer's. Once the state is fresh, whether a message
    // applies depends only on the two documents, so a second failure at the
    // same point would only fail again.
    failed_at: Option<FailurePoint>,
    holding: bool,
}

type FailurePoint = (Vec<ChangeHash>, Option<Vec<ChangeHash>>);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SharePolicyState {
    Unknown,
    Loading,
    Ready(ShareConfig),
}

impl PeerDocConnection {
    pub(super) fn new(peer_id: PeerId, connection_id: ConnectionId) -> Self {
        Self {
            connection_id,
            peer_id,
            sync_state: sync::State::new_read_only(),
            has_requested: false,
            state: PeerDocState::empty(),
            dirty: false,
            sync_due: true,
            heads_at_last_generate: None,
            #[cfg(test)]
            generate_calls: 0,
            share_policy: SharePolicyState::Unknown,
            last_share_config: None,
            pending_sync_messages: Vec::new(),
            peer_heads: None,
            failed_at: None,
            holding: false,
        }
    }

    pub(super) fn reset_sync_state(&mut self) {
        self.sync_state = if self.can_write() {
            sync::State::new()
        } else {
            sync::State::new_read_only()
        };
        self.sync_due = true;
        self.failed_at = None;
        self.holding = false;
    }

    pub(super) fn prepare_request(&mut self) {
        // Initial requests already have a fresh state. Automerge has no explicit
        // API to restart synchronization, so for retries we force a read-only to
        // read-write transition. This resets local bookkeeping and tells the
        // remote to resend changes; replacing the state alone would not do that.
        if self.state.last_sent.is_some() || self.state.last_received.is_some() {
            self.sync_state.set_read_only(true);
            self.sync_state.set_read_only(false);
            self.state.shared_heads = Some(self.sync_state.shared_heads.clone());
            self.state.their_heads = self.sync_state.their_heads.clone();
            self.state.peer_read_only = self.sync_state.is_peer_read_only();
            self.dirty = true;
        }
        self.sync_due = true;
        self.forget_failures();
    }

    pub(super) fn mark_requested(&mut self) {
        if !self.has_requested {
            self.has_requested = true;
            self.dirty = true; // Mark as dirty since the request status changed
            self.sync_due = true;
        }
    }

    pub(super) fn receive_sync_message(
        &mut self,
        now: UnixTimestamp,
        doc: &mut Automerge,
        msg: sync::Message,
    ) -> Result<Duration, AutomergeError> {
        let start = Instant::now();
        let heads = msg.heads.clone();
        self.peer_heads = Some(heads.clone());
        if let Err(e) = doc.receive_sync_message(&mut self.sync_state, msg) {
            self.lost_message(doc);
            return Err(e);
        }
        let duration = start.elapsed();
        if heads
            .iter()
            .all(|h| doc.get_change_meta_by_hash(h).is_some())
        {
            self.failed_at = None;
            self.holding = false;
        }
        self.dirty = true;
        self.sync_due = true;
        self.state.last_received = Some(now);
        self.state.last_acked_heads = self.sync_state.their_heads.clone();
        self.state.shared_heads = Some(self.sync_state.shared_heads.clone());
        self.state.their_heads = self.sync_state.their_heads.clone();
        self.state.peer_read_only = self.sync_state.is_peer_read_only();
        Ok(duration)
    }

    /// A message from the peer failed to decode or apply. Start the sync
    /// state over, once for each pair of document states it fails at.
    pub(super) fn lost_message(&mut self, doc: &Automerge) {
        let point = self.failure_point(doc);
        if self.failed_at.as_ref() == Some(&point) {
            self.holding = true;
            return;
        }
        self.start_over();
        self.failed_at = Some(point);
    }

    fn failure_point(&self, doc: &Automerge) -> FailurePoint {
        (doc.get_heads(), self.peer_heads.clone())
    }

    /// Something outside the two documents changed, so a failure may not
    /// repeat: any document states may be tried again.
    fn forget_failures(&mut self) {
        self.failed_at = None;
        if self.holding {
            self.start_over();
        }
    }

    fn start_over(&mut self) {
        self.holding = false;
        self.failed_at = None;
        let old = std::mem::take(&mut self.sync_state);
        self.sync_state = sync::State {
            their_capabilities: old.their_capabilities,
            read_only: old.read_only,
            peer_read_only: old.peer_read_only,
            needs_reset: true,
            ..sync::State::new()
        };
        self.sync_due = true;
    }

    /// Generate messages in wire order, with timing for each message.
    pub(super) fn generate_sync_messages(
        &mut self,
        now: UnixTimestamp,
        doc: &Automerge,
    ) -> Vec<(sync::Message, Duration)> {
        if self.holding {
            let point = self.failure_point(doc);
            if self.failed_at.as_ref() == Some(&point) {
                return Vec::new();
            }
            self.start_over();
            self.failed_at = Some(point);
        }
        let heads = doc.get_heads();
        let mut messages = Vec::new();
        if !self.sync_due && self.heads_at_last_generate.as_ref() == Some(&heads) {
            return messages;
        }
        self.sync_due = false;
        self.heads_at_last_generate = Some(heads.clone());
        loop {
            let start = Instant::now();
            #[cfg(test)]
            {
                self.generate_calls += 1;
            }
            let Some(msg) = doc.generate_sync_message(&mut self.sync_state) else {
                break;
            };
            let duration = start.elapsed();
            let follow_up = msg.heads != heads;
            if follow_up {
                // Automerge 0.12's fallback reset advertises empty heads but
                // records the real heads as sent. An empty peer may not reply.
                // Correct the bookkeeping to what we actually advertised, then
                // immediately generate the real-heads follow-up in this batch.
                self.sync_state.last_sent_heads.clone_from(&msg.heads);
            }
            self.state.last_sent = Some(now);
            self.state.last_sent_heads = Some(msg.heads.clone());
            self.state.shared_heads = Some(self.sync_state.shared_heads.clone());
            self.dirty = true;
            messages.push((msg, duration));
            if !follow_up {
                break;
            }
        }
        messages
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

    /// Bound buffering while an application-provided policy future is pending.
    /// Permit one oversized message so a legitimate full-document sync is not
    /// limited by this queue's byte budget; transports should bound frame size.
    pub(super) fn buffer_sync_message(
        &mut self,
        msg: SyncMessage,
        received_at: UnixTimestamp,
    ) -> bool {
        const MAX_MESSAGES: usize = 64;
        const MAX_BYTES: usize = 16 * 1024 * 1024;
        let message_bytes = |msg: &SyncMessage| match msg {
            SyncMessage::Sync { data } | SyncMessage::Request { data } => data.len(),
            SyncMessage::DocUnavailable => 0,
        };
        let queued_bytes = self
            .pending_sync_messages
            .iter()
            .fold(0usize, |bytes, (msg, _)| {
                bytes.saturating_add(message_bytes(msg))
            });
        if self.pending_sync_messages.len() >= MAX_MESSAGES
            || (!self.pending_sync_messages.is_empty()
                && queued_bytes.saturating_add(message_bytes(&msg)) > MAX_BYTES)
        {
            return false;
        }
        self.pending_sync_messages.push((msg, received_at));
        true
    }

    pub(super) fn share_config(&self) -> Option<ShareConfig> {
        match self.share_policy {
            SharePolicyState::Ready(config) => Some(config),
            _ => None,
        }
    }

    pub(super) fn can_read(&self) -> bool {
        self.share_config().is_some_and(|config| config.can_read())
    }

    pub(super) fn can_write(&self) -> bool {
        self.share_config().is_some_and(|config| config.can_write())
    }

    pub(super) fn can_send(&self) -> bool {
        self.can_read()
            && (self.has_requested
                || self.sync_state.their_heads.is_some()
                || self.share_config().is_some_and(|config| config.announce))
    }

    pub(super) fn set_share_config(&mut self, config: ShareConfig) -> Option<ShareConfig> {
        let previous = self.last_share_config.replace(config);
        self.share_policy = SharePolicyState::Ready(config);
        if self.state.last_sent.is_none() && self.state.last_received.is_none() {
            // Initialize sessions which have not exchanged any sync messages yet.
            self.reset_sync_state();
        } else {
            if previous.is_some_and(|old| !old.can_read()) && config.can_read() {
                // Denial may have dropped messages; restart the exchange on re-grant.
                self.sync_state.set_read_only(false);
            }
            self.sync_state.set_read_only(!config.can_write());
            self.state.shared_heads = Some(self.sync_state.shared_heads.clone());
            self.state.their_heads = self.sync_state.their_heads.clone();
            self.state.peer_read_only = self.sync_state.is_peer_read_only();
        }
        self.state.share_config = Some(config);
        self.dirty = true;
        self.sync_due = true;
        if previous != Some(config) {
            self.forget_failures();
        }
        previous
    }
}
