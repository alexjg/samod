use super::*;
use crate::{DocumentId, actors::document::phase::request::Request};

fn document() -> Automerge {
    let mut doc = Automerge::new();
    doc.empty_commit(Default::default());
    doc
}

fn connection() -> PeerDocConnection {
    PeerDocConnection::new("remote".into(), ConnectionId::new())
}

#[test]
fn ordinary_generation_returns_one_message_then_an_empty_batch() {
    let mut conn = connection();
    conn.set_share_config(ShareConfig::read_write());
    let doc = document();
    let now = UnixTimestamp::now();
    let messages = conn.generate_sync_messages(now, &doc);
    assert_eq!(messages.len(), 1);
    assert_eq!(messages[0].0.heads, doc.get_heads());
    let state = conn.pop().unwrap();
    assert_eq!(state.last_sent, Some(now));
    assert_eq!(state.last_sent_heads, Some(doc.get_heads()));
    assert!(conn.generate_sync_messages(now, &doc).is_empty());
    assert!(conn.pop().is_none());
}

#[test]
fn fallback_reset_batch_converges_with_an_empty_legacy_peer() {
    let now = UnixTimestamp::now();
    let mut doc = document();
    let mut remote = Automerge::new();
    let mut remote_state = sync::State::new();
    let mut conn = connection();
    conn.set_share_config(ShareConfig::read_only());
    let mut hello = remote.generate_sync_message(&mut remote_state).unwrap();
    hello.flags = None;
    conn.receive_sync_message(now, &mut doc, hello).unwrap();
    conn.set_share_config(ShareConfig::read_write());

    let messages = conn.generate_sync_messages(now, &doc);
    assert_eq!(messages.len(), 2);
    assert!(messages[0].0.heads.is_empty());
    assert_eq!(messages[1].0.heads, doc.get_heads());
    assert_eq!(conn.state().last_sent_heads, Some(doc.get_heads()));
    let mut messages = messages.into_iter();
    remote
        .receive_sync_message(&mut remote_state, messages.next().unwrap().0)
        .unwrap();
    assert!(remote.generate_sync_message(&mut remote_state).is_none());
    assert_ne!(doc.get_heads(), remote.get_heads());
    remote
        .receive_sync_message(&mut remote_state, messages.next().unwrap().0)
        .unwrap();

    for _ in 0..20 {
        let reply = remote.generate_sync_message(&mut remote_state);
        let replied = reply.is_some();
        if let Some(mut reply) = reply {
            reply.flags = None;
            conn.receive_sync_message(now, &mut doc, reply).unwrap();
        }
        let messages = conn.generate_sync_messages(now, &doc);
        if !replied && messages.is_empty() {
            assert_eq!(doc.get_heads(), remote.get_heads());
            return;
        }
        for (message, _) in messages {
            remote
                .receive_sync_message(&mut remote_state, message)
                .unwrap();
        }
    }
    panic!("sync did not become quiescent");
}

#[test]
fn advertised_reset_support_needs_only_one_message() {
    let mut conn = connection();
    conn.set_share_config(ShareConfig::read_only());
    let mut doc = document();
    let now = UnixTimestamp::now();
    let hello = Automerge::new()
        .generate_sync_message(&mut sync::State::new())
        .unwrap();
    conn.receive_sync_message(now, &mut doc, hello).unwrap();
    conn.set_share_config(ShareConfig::read_write());

    let messages = conn.generate_sync_messages(now, &doc);
    assert_eq!(messages.len(), 1);
    assert_eq!(messages[0].0.heads, doc.get_heads());
    assert!(
        messages[0]
            .0
            .flags
            .unwrap()
            .contains(sync::MessageFlags::SYNC_RESET)
    );
    assert!(conn.generate_sync_messages(now, &doc).is_empty());
}

#[test]
fn empty_document_needs_no_follow_up_after_a_fallback_reset() {
    let mut conn = connection();
    conn.set_share_config(ShareConfig::read_only());
    let mut doc = Automerge::new();
    let now = UnixTimestamp::now();
    let mut hello = Automerge::new()
        .generate_sync_message(&mut sync::State::new())
        .unwrap();
    hello.flags = None;
    conn.receive_sync_message(now, &mut doc, hello).unwrap();
    conn.set_share_config(ShareConfig::read_write());

    let messages = conn.generate_sync_messages(now, &doc);
    assert_eq!(messages.len(), 1);
    assert!(messages[0].0.heads.is_empty());
    assert!(conn.generate_sync_messages(now, &doc).is_empty());
}

#[test]
fn only_the_first_message_of_a_request_batch_initiates_a_request() {
    let mut conn = connection();
    conn.set_share_config(ShareConfig::read_only());
    let mut doc = document();
    let now = UnixTimestamp::now();
    let mut hello = Automerge::new()
        .generate_sync_message(&mut sync::State::new())
        .unwrap();
    hello.flags = None;
    conn.receive_sync_message(now, &mut doc, hello).unwrap();
    conn.set_share_config(ShareConfig::read_write());
    let mut request = Request::new(DocumentId::new(&mut rand::rng()), std::iter::once(&conn));

    let messages = request.generate_messages(now, &doc, &mut conn);
    assert_eq!(messages.len(), 2);
    let SyncMessage::Request { data } = &messages[0].0 else {
        panic!("first message must initiate the request");
    };
    assert!(sync::Message::decode(data).unwrap().heads.is_empty());
    let SyncMessage::Sync { data } = &messages[1].0 else {
        panic!("subsequent messages must continue sync");
    };
    assert_eq!(sync::Message::decode(data).unwrap().heads, doc.get_heads());
}
