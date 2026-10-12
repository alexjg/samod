use automerge::{Automerge, ROOT, ReadDoc, transaction::Transactable};
use samod_core::{DocSearchPhase, ShareConfig};
use samod_test_harness::Network;

fn put(doc: &mut Automerge, key: &str) {
    doc.transact::<_, _, automerge::AutomergeError>(|tx| tx.put(ROOT, key, true))
        .unwrap();
}

#[test]
fn granting_access_serves_previously_denied_requests_without_announcing() {
    for grant in [ShareConfig::read_only(), ShareConfig::read_write()] {
        let mut network = Network::new();
        let server = network.create_samod("server");
        let client = network.create_samod("client");
        network
            .samod(&server)
            .set_share_policy(Box::new(|_, _| ShareConfig::none()));
        let doc = network.samod(&server).create_document();
        network.connect(server, client);
        network.run_until_quiescent();
        assert!(network.samod(&client).find_document(&doc.doc_id).is_none());
        let client_id = network.samod(&client).peer_id();
        network.samod(&server).update_share_policy(
            doc.doc_id.clone(),
            client_id,
            grant.with_announce(false),
        );
        assert!(network.samod(&client).is_document_available(&doc.doc_id));
    }
}

#[test]
fn live_write_grants_recover_changes_ignored_during_read_only_and_denied_access() {
    let mut network = Network::new();
    let server = network.create_samod("server");
    let client = network.create_samod("client");
    let doc = network.samod(&server).create_document();
    network.connect(server, client);
    network.run_until_quiescent();
    assert!(network.samod(&client).find_document(&doc.doc_id).is_some());
    let client_id = network.samod(&client).peer_id();
    for (config, key) in [
        (ShareConfig::read_only(), "while_read_only"),
        (ShareConfig::none(), "while_denied"),
    ] {
        network
            .samod(&server)
            .update_share_policy(doc.doc_id.clone(), client_id.clone(), config);
        network
            .samod(&client)
            .with_document(&doc.doc_id, |doc| put(doc, key))
            .unwrap();
        network.run_until_quiescent();
        assert!(
            network
                .samod(&server)
                .document(&doc.doc_id)
                .unwrap()
                .get(ROOT, key)
                .unwrap()
                .is_none()
        );
        network.samod(&server).update_share_policy(
            doc.doc_id.clone(),
            client_id.clone(),
            ShareConfig::read_write().with_announce(false),
        );
        assert!(
            network
                .samod(&server)
                .document(&doc.doc_id)
                .unwrap()
                .get(ROOT, key)
                .unwrap()
                .is_some()
        );
    }
}

#[test]
fn granting_write_access_resumes_blocked_document_searches() {
    for announce in [true, false] {
        let mut network = Network::new();
        let server = network.create_samod("server");
        let client = network.create_samod("client");
        network
            .samod(&server)
            .set_share_policy(Box::new(|_, _| ShareConfig::read_only()));
        let doc = network.samod(&client).create_document();
        network.connect(server, client);
        network.run_until_quiescent();
        assert!(network.samod(&server).find_document(&doc.doc_id).is_none());
        let client_id = network.samod(&client).peer_id();
        network.samod(&server).update_share_policy(
            doc.doc_id.clone(),
            client_id,
            ShareConfig::read_write().with_announce(announce),
        );
        assert!(network.samod(&server).is_document_available(&doc.doc_id));
        assert!(matches!(
            network
                .samod(&server)
                .search_status(&doc.doc_id)
                .unwrap()
                .phase(),
            DocSearchPhase::Ready
        ));
    }
}

#[test]
fn announcement_changes_wake_requests_and_do_not_revoke_existing_sync() {
    let mut network = Network::new();
    let server = network.create_samod("server");
    let client = network.create_samod("client");
    network.samod(&server).set_share_policy(Box::new(|_, _| {
        ShareConfig::read_write().with_announce(false)
    }));
    network.samod(&client).set_share_policy(Box::new(|_, _| {
        ShareConfig::read_write().with_announce(false)
    }));
    let doc = network.samod(&server).create_document();
    network.connect(server, client);
    network.run_until_quiescent();
    assert!(network.samod(&client).find_document(&doc.doc_id).is_none());
    let server_id = network.samod(&server).peer_id();
    network.samod(&client).update_share_policy(
        doc.doc_id.clone(),
        server_id.clone(),
        ShareConfig::read_write(),
    );
    assert!(network.samod(&client).is_document_available(&doc.doc_id));
    network.samod(&client).update_share_policy(
        doc.doc_id.clone(),
        server_id,
        ShareConfig::read_write().with_announce(false),
    );
    network
        .samod(&server)
        .with_document(&doc.doc_id, |doc| put(doc, "later"))
        .unwrap();
    network.run_until_quiescent();
    assert!(
        network
            .samod(&client)
            .document(&doc.doc_id)
            .unwrap()
            .get(ROOT, "later")
            .unwrap()
            .is_some()
    );
}

#[test]
fn update_is_scoped_and_does_not_load_inactive_documents() {
    let mut network = Network::new();
    let server = network.create_samod("server");
    let alice = network.create_samod("alice");
    let bob = network.create_samod("bob");
    network
        .samod(&server)
        .set_share_policy(Box::new(|_, _| ShareConfig::none()));
    let first = network.samod(&server).create_document();
    let second = network.samod(&server).create_document();
    network.connect(server, alice);
    network.connect(server, bob);
    network.run_until_quiescent();
    let alice_id = network.samod(&alice).peer_id();
    network.samod(&server).update_share_policy(
        first.doc_id.clone(),
        alice_id.clone(),
        ShareConfig::read_write(),
    );
    assert!(network.samod(&alice).is_document_available(&first.doc_id));
    assert!(!network.samod(&alice).is_document_available(&second.doc_id));
    assert!(!network.samod(&bob).is_document_available(&first.doc_id));
    let count = network.samod(&server).actor_count();
    network.samod(&server).update_share_policy(
        samod_core::DocumentId::new(&mut rand::rng()),
        alice_id,
        ShareConfig::read_write(),
    );
    assert_eq!(network.samod(&server).actor_count(), count);
}
