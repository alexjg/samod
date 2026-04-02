//! Incremental sync: subscription-based push of changes to peers.
//!
//! After a batch sync establishes baseline state, peers subscribe to
//! receive live updates. When new commits or fragments arrive (from
//! any source), they are forwarded to all subscribed peers.
//!
//! The main entry point is [`IncrementalSync`], which ties together
//! subscription tracking, per-peer monotonic counters, and remote
//! heads staleness filtering.

use std::collections::{HashMap, HashSet};

use sedimentree_core::{
    blob::Blob,
    crypto::digest::Digest,
    fragment::Fragment,
    id::SedimentreeId,
    loose_commit::LooseCommit,
};
use subduction_crypto::signed::Signed;

use crate::{
    messages::SyncMessage,
    types::{PeerId, RemoteHeads},
};

// ---------------------------------------------------------------------------
// SubscriptionTracker
// ---------------------------------------------------------------------------

/// Tracks which peers are subscribed to which sedimentrees.
#[derive(Debug, Clone, Default)]
pub struct SubscriptionTracker {
    subscriptions: HashMap<SedimentreeId, HashSet<PeerId>>,
}

impl SubscriptionTracker {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn subscribe(&mut self, peer: PeerId, id: SedimentreeId) {
        self.subscriptions.entry(id).or_default().insert(peer);
    }

    pub fn unsubscribe(&mut self, peer: PeerId, ids: &[SedimentreeId]) {
        for id in ids {
            if let Some(peers) = self.subscriptions.get_mut(id) {
                peers.remove(&peer);
                if peers.is_empty() {
                    self.subscriptions.remove(id);
                }
            }
        }
    }

    /// Remove a peer from all subscriptions (e.g., on disconnect).
    pub fn peer_disconnected(&mut self, peer: PeerId) {
        self.subscriptions.retain(|_, peers| {
            peers.remove(&peer);
            !peers.is_empty()
        });
    }

    /// Iterate over subscribers for a sedimentree.
    pub fn subscribers(&self, id: &SedimentreeId) -> impl Iterator<Item = &PeerId> {
        self.subscriptions
            .get(id)
            .into_iter()
            .flat_map(|s| s.iter())
    }

    /// Get subscribers for a sedimentree, excluding a specific peer.
    pub fn subscribers_except(&self, id: &SedimentreeId, exclude: PeerId) -> Vec<PeerId> {
        self.subscribers(id)
            .filter(|p| **p != exclude)
            .copied()
            .collect()
    }

    pub fn is_subscribed(&self, peer: &PeerId, id: &SedimentreeId) -> bool {
        self.subscriptions
            .get(id)
            .is_some_and(|peers| peers.contains(peer))
    }
}

// ---------------------------------------------------------------------------
// PeerCounter
// ---------------------------------------------------------------------------

/// Per-peer monotonic send counter.
///
/// Each peer gets an independent counter that starts at 1 on first use.
/// The counter is attached to outgoing messages so the receiver can
/// detect stale/out-of-order messages.
#[derive(Debug, Clone, Default)]
pub struct PeerCounter {
    counters: HashMap<PeerId, u64>,
}

impl PeerCounter {
    pub fn new() -> Self {
        Self::default()
    }

    /// Get the next counter value for a peer (starts at 1, monotonically increasing).
    pub fn next(&mut self, peer: PeerId) -> u64 {
        let counter = self.counters.entry(peer).or_insert(0);
        *counter += 1;
        *counter
    }

    /// Reset the counter for a peer (e.g., on disconnect).
    pub fn clear_peer(&mut self, peer: PeerId) {
        self.counters.remove(&peer);
    }
}

// ---------------------------------------------------------------------------
// RemoteHeadsTracker
// ---------------------------------------------------------------------------

/// Filters stale remote heads updates using per-peer monotonic counters.
///
/// Each peer's messages carry a counter. If an incoming counter is not
/// strictly greater than the last seen value for that peer, the update
/// is stale and should be dropped.
#[derive(Debug, Clone, Default)]
pub struct RemoteHeadsTracker {
    last_seen: HashMap<PeerId, u64>,
}

impl RemoteHeadsTracker {
    pub fn new() -> Self {
        Self::default()
    }

    /// Process an incoming heads update. Returns `Some(heads)` if fresh,
    /// `None` if stale (counter <= last seen).
    pub fn update(&mut self, peer: PeerId, heads: RemoteHeads) -> Option<RemoteHeads> {
        let last = self.last_seen.entry(peer).or_insert(0);
        if heads.counter > *last {
            *last = heads.counter;
            Some(heads)
        } else {
            None
        }
    }

    /// Reset tracking for a peer (e.g., on disconnect).
    pub fn clear_peer(&mut self, peer: PeerId) {
        self.last_seen.remove(&peer);
    }
}

// ---------------------------------------------------------------------------
// IncrementalSync
// ---------------------------------------------------------------------------

/// The result of an incremental sync operation.
#[derive(Debug, Clone, Default)]
pub struct IncrementalStep {
    /// Messages to send, each targeted at a specific peer.
    pub messages: Vec<(PeerId, SyncMessage)>,
    /// Fresh remote heads updates that passed the staleness filter.
    pub remote_heads_updates: Vec<(PeerId, SedimentreeId, RemoteHeads)>,
}

/// Coordinates subscription-based incremental sync.
///
/// Ties together subscription tracking, send counters, and remote heads
/// filtering. The caller is responsible for actually sending messages and
/// performing storage operations — this type only computes what to do.
#[derive(Debug, Clone, Default)]
pub struct IncrementalSync {
    pub subscriptions: SubscriptionTracker,
    send_counter: PeerCounter,
    recv_tracker: RemoteHeadsTracker,
}

impl IncrementalSync {
    pub fn new() -> Self {
        Self::default()
    }

    /// A new commit was received from a remote peer and successfully stored.
    ///
    /// Returns:
    /// - Forward messages to all subscribers (excluding the sender)
    /// - A `HeadsUpdate` message back to the sender
    /// - Any fresh remote heads updates from the sender
    pub fn on_received_commit(
        &mut self,
        from_peer: PeerId,
        id: SedimentreeId,
        commit: Signed<LooseCommit>,
        blob: Blob,
        sender_heads: RemoteHeads,
        current_heads: Vec<Digest<LooseCommit>>,
    ) -> IncrementalStep {
        let mut step = IncrementalStep::default();

        // Process sender's heads through staleness filter
        if let Some(fresh_heads) = self.recv_tracker.update(from_peer, sender_heads) {
            step.remote_heads_updates
                .push((from_peer, id, fresh_heads));
        }

        // Forward to subscribers (excluding sender)
        for subscriber in self.subscriptions.subscribers_except(&id, from_peer) {
            let counter = self.send_counter.next(subscriber);
            step.messages.push((
                subscriber,
                SyncMessage::LooseCommit {
                    id,
                    commit: commit.clone(),
                    blob: blob.clone(),
                    sender_heads: RemoteHeads {
                        counter,
                        heads: current_heads.clone(),
                    },
                },
            ));
        }

        // Send HeadsUpdate back to sender
        let counter = self.send_counter.next(from_peer);
        step.messages.push((
            from_peer,
            SyncMessage::HeadsUpdate {
                id,
                heads: RemoteHeads {
                    counter,
                    heads: current_heads,
                },
            },
        ));

        step
    }

    /// A new fragment was received from a remote peer and successfully stored.
    pub fn on_received_fragment(
        &mut self,
        from_peer: PeerId,
        id: SedimentreeId,
        fragment: Signed<Fragment>,
        blob: Blob,
        sender_heads: RemoteHeads,
        current_heads: Vec<Digest<LooseCommit>>,
    ) -> IncrementalStep {
        let mut step = IncrementalStep::default();

        if let Some(fresh_heads) = self.recv_tracker.update(from_peer, sender_heads) {
            step.remote_heads_updates
                .push((from_peer, id, fresh_heads));
        }

        for subscriber in self.subscriptions.subscribers_except(&id, from_peer) {
            let counter = self.send_counter.next(subscriber);
            step.messages.push((
                subscriber,
                SyncMessage::Fragment {
                    id,
                    fragment: fragment.clone(),
                    blob: blob.clone(),
                    sender_heads: RemoteHeads {
                        counter,
                        heads: current_heads.clone(),
                    },
                },
            ));
        }

        let counter = self.send_counter.next(from_peer);
        step.messages.push((
            from_peer,
            SyncMessage::HeadsUpdate {
                id,
                heads: RemoteHeads {
                    counter,
                    heads: current_heads,
                },
            },
        ));

        step
    }

    /// A local change was made. Forward to all subscribers.
    pub fn on_local_commit(
        &mut self,
        id: SedimentreeId,
        commit: Signed<LooseCommit>,
        blob: Blob,
        current_heads: Vec<Digest<LooseCommit>>,
    ) -> IncrementalStep {
        let mut step = IncrementalStep::default();

        let subscribers: Vec<_> = self.subscriptions.subscribers(&id).copied().collect();
        for subscriber in subscribers {
            let counter = self.send_counter.next(subscriber);
            step.messages.push((
                subscriber,
                SyncMessage::LooseCommit {
                    id,
                    commit: commit.clone(),
                    blob: blob.clone(),
                    sender_heads: RemoteHeads {
                        counter,
                        heads: current_heads.clone(),
                    },
                },
            ));
        }

        step
    }

    /// A local fragment was created. Forward to all subscribers.
    pub fn on_local_fragment(
        &mut self,
        id: SedimentreeId,
        fragment: Signed<Fragment>,
        blob: Blob,
    ) -> IncrementalStep {
        let mut step = IncrementalStep::default();

        let subscribers: Vec<_> = self.subscriptions.subscribers(&id).copied().collect();
        for subscriber in subscribers {
            let counter = self.send_counter.next(subscriber);
            step.messages.push((
                subscriber,
                SyncMessage::Fragment {
                    id,
                    fragment: fragment.clone(),
                    blob: blob.clone(),
                    sender_heads: RemoteHeads {
                        counter,
                        heads: vec![], // fragments don't update heads directly
                    },
                },
            ));
        }

        step
    }

    /// A HeadsUpdate was received from a peer.
    pub fn on_heads_update(
        &mut self,
        from_peer: PeerId,
        id: SedimentreeId,
        heads: RemoteHeads,
    ) -> IncrementalStep {
        let mut step = IncrementalStep::default();

        if let Some(fresh_heads) = self.recv_tracker.update(from_peer, heads) {
            step.remote_heads_updates
                .push((from_peer, id, fresh_heads));
        }

        step
    }

    // --- Subscription management ---

    pub fn subscribe(&mut self, peer: PeerId, id: SedimentreeId) {
        self.subscriptions.subscribe(peer, id);
    }

    pub fn unsubscribe(&mut self, peer: PeerId, ids: &[SedimentreeId]) {
        self.subscriptions.unsubscribe(peer, ids);
    }

    /// Clean up all state for a disconnected peer.
    pub fn peer_disconnected(&mut self, peer: PeerId) {
        self.subscriptions.peer_disconnected(peer);
        self.send_counter.clear_peer(peer);
        self.recv_tracker.clear_peer(peer);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sedimentree_core::{
        blob::BlobMeta,
        codec::{encode::EncodeFields, schema::Schema},
        crypto::digest::Digest,
    };

    fn peer(val: u8) -> PeerId {
        PeerId::new([val; 32])
    }

    fn sed_id(val: u8) -> SedimentreeId {
        SedimentreeId::new([val; 32])
    }

    fn make_signed_commit(seed: u8) -> (Signed<LooseCommit>, Blob) {
        let signing_key = ed25519_dalek::SigningKey::from_bytes(&[seed; 32]);
        let vk = signing_key.verifying_key();
        let blob = Blob::new(vec![seed; 4]);
        let commit = LooseCommit::new(
            sed_id(1),
            std::collections::BTreeSet::new(),
            BlobMeta::new(&blob),
        );
        let mut payload = Vec::new();
        payload.extend_from_slice(&LooseCommit::SCHEMA);
        payload.extend_from_slice(vk.as_bytes());
        commit.encode_fields(&mut payload);
        use ed25519_dalek::Signer as _;
        let sig = signing_key.sign(&payload);
        (Signed::from_parts(vk, sig, &commit), blob)
    }

    // --- SubscriptionTracker ---

    #[test]
    fn subscription_tracker_basic() {
        let mut tracker = SubscriptionTracker::new();
        let p1 = peer(1);
        let p2 = peer(2);
        let s1 = sed_id(1);
        let s2 = sed_id(2);

        tracker.subscribe(p1, s1);
        tracker.subscribe(p2, s1);
        tracker.subscribe(p1, s2);

        assert!(tracker.is_subscribed(&p1, &s1));
        assert!(tracker.is_subscribed(&p2, &s1));
        assert!(tracker.is_subscribed(&p1, &s2));
        assert!(!tracker.is_subscribed(&p2, &s2));

        let subs: Vec<_> = tracker.subscribers(&s1).copied().collect();
        assert_eq!(subs.len(), 2);

        let except_p1 = tracker.subscribers_except(&s1, p1);
        assert_eq!(except_p1, vec![p2]);
    }

    #[test]
    fn subscription_tracker_unsubscribe() {
        let mut tracker = SubscriptionTracker::new();
        let p1 = peer(1);
        let s1 = sed_id(1);
        let s2 = sed_id(2);

        tracker.subscribe(p1, s1);
        tracker.subscribe(p1, s2);
        tracker.unsubscribe(p1, &[s1]);

        assert!(!tracker.is_subscribed(&p1, &s1));
        assert!(tracker.is_subscribed(&p1, &s2));
    }

    #[test]
    fn subscription_tracker_peer_disconnect() {
        let mut tracker = SubscriptionTracker::new();
        let p1 = peer(1);
        let p2 = peer(2);
        let s1 = sed_id(1);

        tracker.subscribe(p1, s1);
        tracker.subscribe(p2, s1);
        tracker.peer_disconnected(p1);

        assert!(!tracker.is_subscribed(&p1, &s1));
        assert!(tracker.is_subscribed(&p2, &s1));
    }

    // --- PeerCounter ---

    #[test]
    fn peer_counter_monotonic() {
        let mut counter = PeerCounter::new();
        let p1 = peer(1);
        let p2 = peer(2);

        assert_eq!(counter.next(p1), 1);
        assert_eq!(counter.next(p1), 2);
        assert_eq!(counter.next(p1), 3);
        assert_eq!(counter.next(p2), 1);
        assert_eq!(counter.next(p2), 2);
    }

    #[test]
    fn peer_counter_clear_resets() {
        let mut counter = PeerCounter::new();
        let p1 = peer(1);

        assert_eq!(counter.next(p1), 1);
        assert_eq!(counter.next(p1), 2);
        counter.clear_peer(p1);
        assert_eq!(counter.next(p1), 1);
    }

    // --- RemoteHeadsTracker ---

    #[test]
    fn remote_heads_fresh_accepted() {
        let mut tracker = RemoteHeadsTracker::new();
        let p1 = peer(1);
        let heads = RemoteHeads {
            counter: 1,
            heads: vec![],
        };

        assert!(tracker.update(p1, heads).is_some());
    }

    #[test]
    fn remote_heads_stale_dropped() {
        let mut tracker = RemoteHeadsTracker::new();
        let p1 = peer(1);

        tracker.update(
            p1,
            RemoteHeads {
                counter: 5,
                heads: vec![],
            },
        );

        // Counter <= last seen → stale
        assert!(tracker
            .update(
                p1,
                RemoteHeads {
                    counter: 3,
                    heads: vec![],
                }
            )
            .is_none());
        assert!(tracker
            .update(
                p1,
                RemoteHeads {
                    counter: 5,
                    heads: vec![],
                }
            )
            .is_none());

        // Counter > last seen → fresh
        assert!(tracker
            .update(
                p1,
                RemoteHeads {
                    counter: 6,
                    heads: vec![],
                }
            )
            .is_some());
    }

    // --- IncrementalSync ---

    #[test]
    fn on_received_commit_forwards_to_subscribers() {
        let mut sync = IncrementalSync::new();
        let sender = peer(1);
        let sub_a = peer(2);
        let sub_b = peer(3);
        let id = sed_id(1);

        sync.subscribe(sub_a, id);
        sync.subscribe(sub_b, id);
        sync.subscribe(sender, id); // sender is also subscribed but should be excluded

        let (commit, blob) = make_signed_commit(10);
        let sender_heads = RemoteHeads {
            counter: 1,
            heads: vec![],
        };
        let current_heads = vec![Digest::force_from_bytes([99; 32])];

        let step = sync.on_received_commit(
            sender,
            id,
            commit,
            blob,
            sender_heads,
            current_heads,
        );

        // Should have: 2 forwards (sub_a, sub_b) + 1 HeadsUpdate (sender) = 3 messages
        assert_eq!(step.messages.len(), 3);

        let forward_targets: HashSet<_> = step
            .messages
            .iter()
            .filter(|(_, m)| matches!(m, SyncMessage::LooseCommit { .. }))
            .map(|(p, _)| *p)
            .collect();
        assert!(forward_targets.contains(&sub_a));
        assert!(forward_targets.contains(&sub_b));
        assert!(!forward_targets.contains(&sender));

        // HeadsUpdate sent to sender
        let heads_update = step
            .messages
            .iter()
            .find(|(p, _)| *p == sender)
            .unwrap();
        assert!(matches!(heads_update.1, SyncMessage::HeadsUpdate { .. }));

        // Fresh remote heads update
        assert_eq!(step.remote_heads_updates.len(), 1);
    }

    #[test]
    fn on_local_commit_sends_to_all_subscribers() {
        let mut sync = IncrementalSync::new();
        let sub_a = peer(1);
        let sub_b = peer(2);
        let id = sed_id(1);

        sync.subscribe(sub_a, id);
        sync.subscribe(sub_b, id);

        let (commit, blob) = make_signed_commit(10);
        let current_heads = vec![Digest::force_from_bytes([99; 32])];

        let step = sync.on_local_commit(id, commit, blob, current_heads);

        assert_eq!(step.messages.len(), 2);
        let targets: HashSet<_> = step.messages.iter().map(|(p, _)| *p).collect();
        assert!(targets.contains(&sub_a));
        assert!(targets.contains(&sub_b));

        // No remote heads updates for local changes
        assert!(step.remote_heads_updates.is_empty());
    }

    #[test]
    fn on_heads_update_filters_stale() {
        let mut sync = IncrementalSync::new();
        let p1 = peer(1);
        let id = sed_id(1);

        // First update is fresh
        let step = sync.on_heads_update(
            p1,
            id,
            RemoteHeads {
                counter: 5,
                heads: vec![],
            },
        );
        assert_eq!(step.remote_heads_updates.len(), 1);

        // Stale update (counter <= 5) is dropped
        let step = sync.on_heads_update(
            p1,
            id,
            RemoteHeads {
                counter: 3,
                heads: vec![],
            },
        );
        assert!(step.remote_heads_updates.is_empty());
    }

    #[test]
    fn per_peer_counters_are_independent() {
        let mut sync = IncrementalSync::new();
        let sub_a = peer(1);
        let sub_b = peer(2);
        let id = sed_id(1);

        sync.subscribe(sub_a, id);
        sync.subscribe(sub_b, id);

        let (commit, blob) = make_signed_commit(10);
        let heads = vec![];

        // First local change
        let step = sync.on_local_commit(id, commit.clone(), blob.clone(), heads.clone());
        let counter_a_1 = extract_counter(&step, sub_a);
        let counter_b_1 = extract_counter(&step, sub_b);
        assert_eq!(counter_a_1, 1);
        assert_eq!(counter_b_1, 1);

        // Second local change
        let step = sync.on_local_commit(id, commit, blob, heads);
        let counter_a_2 = extract_counter(&step, sub_a);
        let counter_b_2 = extract_counter(&step, sub_b);
        assert_eq!(counter_a_2, 2);
        assert_eq!(counter_b_2, 2);
    }

    #[test]
    fn peer_disconnect_cleans_up() {
        let mut sync = IncrementalSync::new();
        let p1 = peer(1);
        let id = sed_id(1);

        sync.subscribe(p1, id);
        // Generate some counter state
        let (commit, blob) = make_signed_commit(10);
        sync.on_local_commit(id, commit.clone(), blob.clone(), vec![]);

        // Accept a heads update to create recv_tracker state
        sync.on_heads_update(
            p1,
            id,
            RemoteHeads {
                counter: 1,
                heads: vec![],
            },
        );

        sync.peer_disconnected(p1);

        // Subscription gone
        assert!(!sync.subscriptions.is_subscribed(&p1, &id));

        // Local change produces no messages (no subscribers)
        let step = sync.on_local_commit(id, commit, blob, vec![]);
        assert!(step.messages.is_empty());

        // Heads update with counter=1 is fresh again (tracker was reset)
        let step = sync.on_heads_update(
            p1,
            id,
            RemoteHeads {
                counter: 1,
                heads: vec![],
            },
        );
        assert_eq!(step.remote_heads_updates.len(), 1);
    }

    fn extract_counter(step: &IncrementalStep, peer: PeerId) -> u64 {
        step.messages
            .iter()
            .find_map(|(p, m)| {
                if *p != peer {
                    return None;
                }
                match m {
                    SyncMessage::LooseCommit { sender_heads, .. } => Some(sender_heads.counter),
                    SyncMessage::Fragment { sender_heads, .. } => Some(sender_heads.counter),
                    SyncMessage::HeadsUpdate { heads, .. } => Some(heads.counter),
                    _ => None,
                }
            })
            .expect("no message found for peer")
    }
}
