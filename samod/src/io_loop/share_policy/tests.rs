use super::*;
use futures::executor::block_on;

fn active() -> (Policies, DocumentActorId, DocumentId, PeerId) {
    let mut policies = Policies::default();
    let actor = DocumentActorId::new();
    let doc = DocumentId::new(&mut rand::rng());
    let peer: PeerId = "remote".into();
    policies.set_peers(actor, HashSet::from([peer.clone()]));
    (policies, actor, doc, peer)
}

fn assert_answer(task: &ActorTask, id: IoTaskId, expected: ShareConfig) {
    let ActorTask::IoComplete(IoResult {
        task_id,
        payload: DocumentIoResult::CheckSharePolicy(config),
    }) = task
    else {
        panic!("expected initial policy answer");
    };
    assert_eq!(*task_id, id);
    assert_eq!(*config, expected);
}

#[test]
fn one_subscription_answers_waiting_and_cached_initial_checks() {
    block_on(async {
        let (mut policies, actor, doc, peer) = active();
        let policy = ShareConfig::read_only();
        let first = IoTaskId::from(1usize);
        let second = IoTaskId::from(2usize);
        let Check::Subscribe(events) =
            policies.check(&policy, actor, doc.clone(), peer.clone(), first)
        else {
            panic!("first check must subscribe");
        };
        let mut events = Box::pin(events);
        assert!(matches!(
            policies.check(&policy, actor, doc.clone(), peer.clone(), second),
            Check::Pending
        ));
        let answers = policies.handle_event(events.next().await.unwrap());
        assert_eq!(answers.len(), 2);
        assert_answer(&answers[0], first, policy);
        assert_answer(&answers[1], second, policy);
        // EOF preserves the cache, even after the polling stream is gone.
        assert!(
            policies
                .handle_event(events.next().await.unwrap())
                .is_empty()
        );
        assert!(events.next().await.is_none());
        let third = IoTaskId::from(3usize);
        let Check::Answer(answer) = policies.check(&policy, actor, doc, peer, third) else {
            panic!("subsequent connection must use the cache");
        };
        assert_answer(&answer, third, policy);
    });
}

#[test]
fn changed_decisions_push_messages_and_duplicates_do_nothing() {
    block_on(async {
        let (mut policies, actor, doc, peer) = active();
        let policy = |_, _| {
            stream::iter([
                ShareConfig::read_only(),
                ShareConfig::read_only(),
                ShareConfig::read_write(),
            ])
        };
        let Check::Subscribe(events) =
            policies.check(&policy, actor, doc, peer.clone(), 1usize.into())
        else {
            panic!("expected subscription");
        };
        let mut events = Box::pin(events);
        let first = policies.handle_event(events.next().await.unwrap());
        assert_answer(&first[0], 1usize.into(), ShareConfig::read_only());
        assert!(
            policies
                .handle_event(events.next().await.unwrap())
                .is_empty()
        );
        let update = policies.handle_event(events.next().await.unwrap());
        assert!(matches!(update.as_slice(), [ActorTask::HandleMessage(_)]));
        assert_eq!(
            policies.subscriptions[&(actor, peer)]
                .as_ref()
                .unwrap()
                .latest,
            Some(ShareConfig::read_write())
        );
        assert!(
            policies
                .handle_event(events.next().await.unwrap())
                .is_empty()
        );
    });
}

#[test]
fn empty_stream_denies_pending_checks() {
    block_on(async {
        let (mut policies, actor, doc, peer) = active();
        let policy = |_, _| stream::empty();
        let Check::Subscribe(events) = policies.check(&policy, actor, doc, peer, 1usize.into())
        else {
            panic!("expected subscription");
        };
        let mut events = Box::pin(events);
        let answers = policies.handle_event(events.next().await.unwrap());
        assert_eq!(answers.len(), 1);
        assert_answer(&answers[0], 1usize.into(), ShareConfig::none());
        assert!(events.next().await.is_none());
    });
}

#[test]
fn disconnected_subscriptions_are_cancelled_and_late_events_are_ignored() {
    block_on(async {
        let (mut policies, actor, doc, peer) = active();
        let policy = |_, _| stream::repeat(ShareConfig::read_write());
        let Check::Subscribe(events) =
            policies.check(&policy, actor, doc.clone(), peer.clone(), 1usize.into())
        else {
            panic!("expected subscription");
        };
        let mut events = Box::pin(events);
        let stale = events.next().await.unwrap();
        policies.set_peers(actor, HashSet::new());
        assert!(events.next().await.is_none(), "removal aborts polling");
        policies.set_peers(actor, HashSet::from([peer.clone()]));
        let replacement_policy = ShareConfig::read_only();
        let Check::Subscribe(replacement) =
            policies.check(&replacement_policy, actor, doc, peer.clone(), 2usize.into())
        else {
            panic!("reconnection must subscribe again");
        };
        assert!(policies.handle_event(stale).is_empty());
        assert_eq!(
            policies.subscriptions[&(actor, peer)]
                .as_ref()
                .unwrap()
                .latest,
            None
        );
        let mut replacement = Box::pin(replacement);
        let answers = policies.handle_event(replacement.next().await.unwrap());
        assert_answer(&answers[0], 2usize.into(), replacement_policy);
        policies.clear();
        assert!(
            replacement.next().await.is_none(),
            "shutdown aborts polling"
        );
    });
}
