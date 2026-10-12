//! Policy state belongs to the IO loop. Streams only produce subscription events.

use std::{
    collections::{HashMap, HashSet},
    task::Poll,
};

use futures::{
    Stream, StreamExt,
    future::poll_fn,
    stream::{self, AbortHandle, Abortable},
};
use samod_core::{
    DocumentActorId, DocumentId, PeerId, ShareConfig,
    actors::{HubToDocMsg, document::io::DocumentIoResult},
    io::{IoResult, IoTaskId},
};

use crate::{actor_task::ActorTask, share_policy::LocalSharePolicy};

#[derive(Default)]
pub(super) struct Policies {
    // None means the pair is active but its first check hasn't arrived yet.
    subscriptions: HashMap<(DocumentActorId, PeerId), Option<Subscription>>,
}

struct Subscription {
    // The first check's unique task ID also identifies this subscription.
    id: IoTaskId,
    latest: Option<ShareConfig>,
    waiting: Vec<IoTaskId>,
    abort: AbortHandle,
}

impl Drop for Subscription {
    fn drop(&mut self) {
        self.abort.abort();
    }
}

pub(super) enum Check<S> {
    Answer(ActorTask),
    Pending,
    Subscribe(S),
}

pub(super) struct PolicyEvent {
    pub(super) actor_id: DocumentActorId,
    peer_id: PeerId,
    subscription_id: IoTaskId,
    // None is EOF: retain the last decision, or deny if there wasn't one.
    config: Option<ShareConfig>,
}

impl Policies {
    pub(super) fn set_peers(&mut self, actor_id: DocumentActorId, peers: HashSet<PeerId>) {
        self.subscriptions
            .retain(|(actor, peer), _| *actor != actor_id || peers.contains(peer));
        for peer in peers {
            self.subscriptions.entry((actor_id, peer)).or_default();
        }
    }

    pub(super) fn clear(&mut self) {
        self.subscriptions.clear();
    }

    /// Answer from the cache, wait for the first item, or start a subscription.
    pub(super) fn check<'a, A: LocalSharePolicy>(
        &mut self,
        policy: &'a A,
        actor_id: DocumentActorId,
        document_id: DocumentId,
        peer_id: PeerId,
        task_id: IoTaskId,
    ) -> Check<impl Stream<Item = PolicyEvent> + 'static + use<'a, A>> {
        let Some(slot) = self.subscriptions.get_mut(&(actor_id, peer_id.clone())) else {
            return Check::Pending;
        };
        if let Some(subscription) = slot {
            if let Some(config) = subscription.latest {
                return Check::Answer(answer(task_id, config));
            }
            subscription.waiting.push(task_id);
            return Check::Pending;
        }
        let (abort, registration) = AbortHandle::new_pair();
        *slot = Some(Subscription {
            id: task_id,
            latest: None,
            waiting: vec![task_id],
            abort,
        });
        let events = policy
            .share(document_id, peer_id.clone())
            .map(Some)
            .chain(stream::once(async { None }))
            .map(move |config| PolicyEvent {
                actor_id,
                peer_id: peer_id.clone(),
                subscription_id: task_id,
                config,
            })
            .then(|event| async {
                // Yield even for duplicates so an always-ready stream cannot
                // monopolize the IO loop. This also works on non-Send runtimes.
                yield_now().await;
                event
            });
        Check::Subscribe(Abortable::new(events, registration))
    }

    /// Apply one stream event and return messages for its document actor.
    pub(super) fn handle_event(&mut self, event: PolicyEvent) -> Vec<ActorTask> {
        let Some(Some(subscription)) = self
            .subscriptions
            .get_mut(&(event.actor_id, event.peer_id.clone()))
        else {
            return Vec::new();
        };
        if subscription.id != event.subscription_id {
            // A disconnected peer may already have a new subscription.
            return Vec::new();
        }
        let config = match event.config {
            Some(config) => config,
            None if subscription.latest.is_none() => ShareConfig::none(),
            None => return Vec::new(),
        };
        let previous = subscription.latest.replace(config);
        if previous == Some(config) {
            return Vec::new();
        }
        if previous.is_none() {
            subscription
                .waiting
                .drain(..)
                .map(|task| answer(task, config))
                .collect()
        } else {
            vec![ActorTask::HandleMessage(HubToDocMsg::update_share_policy(
                event.peer_id,
                config,
            ))]
        }
    }
}

async fn yield_now() {
    let mut yielded = false;
    poll_fn(|cx| {
        if std::mem::replace(&mut yielded, true) {
            Poll::Ready(())
        } else {
            cx.waker().wake_by_ref();
            Poll::Pending
        }
    })
    .await;
}

fn answer(task_id: IoTaskId, config: ShareConfig) -> ActorTask {
    ActorTask::IoComplete(IoResult {
        task_id,
        payload: DocumentIoResult::CheckSharePolicy(config),
    })
}

#[cfg(test)]
mod tests;
