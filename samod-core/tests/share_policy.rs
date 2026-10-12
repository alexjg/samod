use automerge::{Automerge, ROOT, ReadDoc, transaction::Transactable};
use samod_core::{DocSearchPhase, PeerRequestState, ShareConfig};
use samod_test_harness::Network;

fn put(doc: &mut Automerge, key: &str) {
    doc.transact::<_, _, automerge::AutomergeError>(|tx| tx.put(ROOT, key, true))
        .unwrap();
}

#[test]
fn read_only_publishes_but_does_not_accept_changes() {
    let mut network = Network::new();
    let publisher = network.create_samod("publisher");
    let reader = network.create_samod("reader");
    network
        .samod(&publisher)
        .set_share_policy(Box::new(|_, _| ShareConfig::read_only()));
    let doc = network.samod(&publisher).create_document();
    let connection = network.connect(publisher, reader);
    network.run_until_quiescent();
    let reader_actor = network.samod(&reader).find_document(&doc.doc_id).unwrap();
    assert!(network.samod(&reader).peer_states(&doc.doc_id)[&connection.right].peer_read_only);
    assert_eq!(
        network.samod(&publisher).peer_states(&doc.doc_id)[&connection.left].share_config,
        Some(ShareConfig::read_only())
    );

    network
        .samod(&reader)
        .with_document(&doc.doc_id, |doc| put(doc, "reader"))
        .unwrap();
    network
        .samod(&publisher)
        .with_document(&doc.doc_id, |doc| put(doc, "publisher"))
        .unwrap();
    network.run_until_quiescent();
    assert!(
        network
            .samod(&publisher)
            .document(&doc.doc_id)
            .unwrap()
            .get(ROOT, "reader")
            .unwrap()
            .is_none()
    );
    assert!(
        network
            .samod(&reader)
            .document(&doc.doc_id)
            .unwrap()
            .get(ROOT, "publisher")
            .unwrap()
            .is_some()
    );

    // Read-only peers can still exchange presence.
    network.samod(&reader).broadcast(reader_actor, vec![1]);
    network.samod(&publisher).broadcast(doc.actor_id, vec![2]);
    network.run_until_quiescent();
    assert_eq!(
        network.samod(&publisher).pop_ephemera(doc.actor_id),
        vec![vec![1]]
    );
    assert_eq!(
        network.samod(&reader).pop_ephemera(reader_actor),
        vec![vec![2]]
    );

    // Reconnecting preserves the policy, not the default read-write sync state.
    network.disconnect(publisher, reader);
    network.connect(publisher, reader);
    network.run_until_quiescent();
    assert!(
        network
            .samod(&publisher)
            .document(&doc.doc_id)
            .unwrap()
            .get(ROOT, "reader")
            .unwrap()
            .is_none()
    );

    // A new connection re-evaluates policy and can accept previously ignored changes.
    network.disconnect(publisher, reader);
    network
        .samod(&publisher)
        .set_share_policy(Box::new(|_, _| ShareConfig::read_write()));
    network.connect(publisher, reader);
    network.run_until_quiescent();
    assert!(
        network
            .samod(&publisher)
            .document(&doc.doc_id)
            .unwrap()
            .get(ROOT, "reader")
            .unwrap()
            .is_some()
    );
}

#[test]
fn non_announcing_read_only_peer_answers_explicit_requests() {
    let mut network = Network::new();
    let publisher = network.create_samod("publisher");
    let reader = network.create_samod("reader");
    network.samod(&publisher).set_share_policy(Box::new(|_, _| {
        ShareConfig::read_only().with_announce(false)
    }));
    let doc = network.samod(&publisher).create_document();
    network.connect(publisher, reader);
    network.run_until_quiescent();
    assert_eq!(network.samod(&reader).actor_count(), 0);
    network.samod(&publisher).broadcast(doc.actor_id, vec![1]);
    network.run_until_quiescent();
    assert_eq!(network.samod(&reader).actor_count(), 0);
    assert!(network.samod(&reader).find_document(&doc.doc_id).is_some());
}

#[test]
fn denied_peer_cannot_request_or_publish_documents() {
    let mut network = Network::new();
    let server = network.create_samod("server");
    let client = network.create_samod("client");
    // Even an explicit announce=true cannot override denied access.
    network
        .samod(&server)
        .set_share_policy(Box::new(|_, _| ShareConfig::none().with_announce(true)));
    let server_doc = network.samod(&server).create_document();
    network.connect(server, client);
    network.run_until_quiescent();
    assert_eq!(network.samod(&client).actor_count(), 0);
    assert!(
        network
            .samod(&client)
            .find_document(&server_doc.doc_id)
            .is_none()
    );
    assert!(
        network
            .samod(&client)
            .search_status(&server_doc.doc_id)
            .unwrap()
            .is_currently_unavailable()
    );

    let client_doc = network.samod(&client).create_document();
    network.run_until_quiescent();
    assert!(
        network
            .samod(&server)
            .find_document(&client_doc.doc_id)
            .is_none()
    );
    assert!(
        network
            .samod(&server)
            .search_status(&client_doc.doc_id)
            .unwrap()
            .is_currently_unavailable()
    );

    // Neither local broadcasts nor incoming ephemera bypass access control.
    let client_actor = network.samod(&client).search_for_doc(&server_doc.doc_id);
    network
        .samod(&server)
        .broadcast(server_doc.actor_id, vec![1]);
    network.samod(&client).broadcast(client_actor, vec![2]);
    network.run_until_quiescent();
    assert!(network.samod(&client).pop_ephemera(client_actor).is_empty());
    assert!(
        network
            .samod(&server)
            .pop_ephemera(server_doc.actor_id)
            .is_empty()
    );
}

#[test]
fn incoming_requests_do_not_make_read_only_sources_look_fetchable() {
    let mut network = Network::new();
    let alice = network.create_samod("alice");
    let bob = network.create_samod("bob");
    network
        .samod(&alice)
        .set_share_policy(Box::new(|_, _| ShareConfig::read_only()));
    network.connect(alice, bob);
    network.run_until_quiescent();
    let missing = samod_core::DocumentId::new(&mut rand::rng());
    assert!(network.samod(&bob).find_document(&missing).is_none());
    assert!(network.samod(&alice).find_document(&missing).is_none());
    assert!(
        network
            .samod(&alice)
            .search_status(&missing)
            .unwrap()
            .is_currently_unavailable()
    );
}

#[test]
fn non_announcing_read_only_peer_responds_after_obtaining_document_elsewhere() {
    let mut network = Network::new();
    let upstream = network.create_samod("upstream");
    let reader = network.create_samod("reader");
    let server = network.create_samod("server");
    let doc = network.samod(&upstream).create_document();
    network.connect(upstream, reader);
    network.run_until_quiescent();
    assert!(network.samod(&reader).find_document(&doc.doc_id).is_some());
    network.disconnect(upstream, reader);
    network
        .samod(&upstream)
        .with_document(&doc.doc_id, |doc| put(doc, "new_upstream"))
        .unwrap();
    network
        .samod(&reader)
        .with_document(&doc.doc_id, |doc| put(doc, "rejected_reader"))
        .unwrap();
    let upstream_id = network.samod(&upstream).peer_id();
    network
        .samod(&server)
        .set_share_policy(Box::new(move |_, peer| {
            if peer == upstream_id {
                ShareConfig::read_write()
            } else {
                ShareConfig::read_only().with_announce(false)
            }
        }));
    network.connect(server, reader);
    network.run_until_quiescent();
    assert!(network.samod(&server).find_document(&doc.doc_id).is_none());
    network.connect(server, upstream);
    network.run_until_quiescent();
    assert!(
        network
            .samod(&server)
            .document(&doc.doc_id)
            .unwrap()
            .get(ROOT, "rejected_reader")
            .unwrap()
            .is_none()
    );
    assert!(
        network
            .samod(&reader)
            .document(&doc.doc_id)
            .unwrap()
            .get(ROOT, "new_upstream")
            .unwrap()
            .is_some()
    );
}

#[test]
fn read_only_sources_are_excluded_from_missing_document_searches() {
    let mut network = Network::new();
    let alice = network.create_samod("alice");
    let bob = network.create_samod("bob");
    network
        .samod(&alice)
        .set_share_policy(Box::new(|_, _| ShareConfig::read_only()));
    let doc = network.samod(&bob).create_document();
    network.connect(alice, bob);
    network.run_until_quiescent();
    assert!(network.samod(&alice).find_document(&doc.doc_id).is_none());
    let alice = network.samod(&alice);
    let status = alice.search_status(&doc.doc_id).unwrap();
    assert!(status.is_currently_unavailable());
    let DocSearchPhase::Searching(peers) = status.phase() else {
        panic!("expected searching")
    };
    assert!(
        peers
            .values()
            .all(|state| *state == PeerRequestState::BlockedByPolicy)
    );
}

#[test]
fn access_is_per_document_and_per_peer() {
    let mut network = Network::new();
    let server = network.create_samod("server");
    let writer = network.create_samod("writer");
    let reader = network.create_samod("reader");
    let shared = network.samod(&server).create_document();
    let private = network.samod(&server).create_document();
    let shared_id = shared.doc_id.clone();
    let writer_id = network.samod(&writer).peer_id();
    network
        .samod(&server)
        .set_share_policy(Box::new(move |doc, peer| {
            if doc != shared_id {
                ShareConfig::none()
            } else if peer == writer_id {
                ShareConfig::read_write()
            } else {
                ShareConfig::read_only()
            }
        }));
    network.connect(server, writer);
    network.connect(server, reader);
    network.run_until_quiescent();
    assert!(
        network
            .samod(&writer)
            .find_document(&shared.doc_id)
            .is_some()
    );
    assert!(
        network
            .samod(&reader)
            .find_document(&shared.doc_id)
            .is_some()
    );
    assert!(
        network
            .samod(&writer)
            .find_document(&private.doc_id)
            .is_none()
    );
    assert!(
        network
            .samod(&reader)
            .find_document(&private.doc_id)
            .is_none()
    );
    network
        .samod(&writer)
        .with_document(&shared.doc_id, |doc| put(doc, "writer"))
        .unwrap();
    network
        .samod(&reader)
        .with_document(&shared.doc_id, |doc| put(doc, "reader"))
        .unwrap();
    network.run_until_quiescent();
    assert!(
        network
            .samod(&server)
            .document(&shared.doc_id)
            .unwrap()
            .get(ROOT, "writer")
            .unwrap()
            .is_some()
    );
    assert!(
        network
            .samod(&server)
            .document(&shared.doc_id)
            .unwrap()
            .get(ROOT, "reader")
            .unwrap()
            .is_none()
    );
    assert!(
        network
            .samod(&reader)
            .document(&shared.doc_id)
            .unwrap()
            .get(ROOT, "writer")
            .unwrap()
            .is_some()
    );
}

#[test]
fn denied_peers_do_not_receive_gossiped_ephemera() {
    let mut network = Network::new();
    let server = network.create_samod("server");
    let allowed = network.create_samod("allowed");
    let denied = network.create_samod("denied");
    let allowed_id = network.samod(&allowed).peer_id();
    network
        .samod(&server)
        .set_share_policy(Box::new(move |_, peer| {
            if peer == allowed_id {
                ShareConfig::read_write()
            } else {
                ShareConfig::none()
            }
        }));
    let doc = network.samod(&server).create_document();
    network.connect(server, allowed);
    network.connect(server, denied);
    network.run_until_quiescent();
    let allowed_actor = network.samod(&allowed).find_document(&doc.doc_id).unwrap();
    let denied_actor = network.samod(&denied).search_for_doc(&doc.doc_id);
    network.run_until_quiescent();
    network.samod(&allowed).broadcast(allowed_actor, vec![1]);
    network.run_until_quiescent();
    assert_eq!(
        network.samod(&server).pop_ephemera(doc.actor_id),
        vec![vec![1]]
    );
    assert!(network.samod(&denied).pop_ephemera(denied_actor).is_empty());
}
