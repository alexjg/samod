#![cfg(feature = "tokio")]

use std::time::Duration;

use automerge::Automerge;
use samod::{PeerId, Repo, storage::InMemoryStorage};
mod tincans;

fn init_logging() {
    let _ = tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .try_init();
}

#[tokio::test]
async fn smoke() {
    init_logging();
    let storage = InMemoryStorage::new();
    let samod = Repo::build_tokio()
        .with_storage(storage.clone())
        .load()
        .await;

    let doc = samod.create(Automerge::new()).await.unwrap();
    doc.with_document(|am| {
        use automerge::{AutomergeError, ROOT};

        am.transact::<_, _, AutomergeError>(|tx| {
            use automerge::transaction::Transactable;

            tx.put(ROOT, "foo", "bar")?;
            Ok(())
        })
        .unwrap();
    });

    let new_samod = Repo::build_tokio().with_storage(storage).load().await;
    let handle2 = new_samod.find(doc.document_id().clone()).await.unwrap();
    assert!(handle2.is_some());
}

#[tokio::test]
async fn basic_sync() {
    init_logging();

    let alice = Repo::build_tokio()
        .with_peer_id(PeerId::from("alice"))
        .load()
        .await;

    let bob = Repo::build_tokio()
        .with_peer_id(PeerId::from("bob"))
        .load()
        .await;

    let _connected = tincans::connect_repos(&alice, &bob).await;

    let alice_handle = alice.create(Automerge::new()).await.unwrap();
    alice_handle.with_document(|am| {
        use automerge::{AutomergeError, ROOT};

        am.transact::<_, _, AutomergeError>(|tx| {
            use automerge::transaction::Transactable;

            tx.put(ROOT, "foo", "bar")?;
            Ok(())
        })
        .unwrap();
    });

    let bob_handle = bob.find(alice_handle.document_id().clone()).await.unwrap();
    assert!(bob_handle.is_some());
    bob.stop().await;
    alice.stop().await;
}

#[tokio::test]
#[cfg(feature = "threadpool")]
async fn basic_sync_threadpool() {
    use samod::ConcurrencyConfig;
    init_logging();

    let alice = Repo::build_tokio()
        .with_peer_id(PeerId::from("alice"))
        .with_concurrency(ConcurrencyConfig::Threadpool(
            rayon::ThreadPoolBuilder::new().build().unwrap(),
        ))
        .load()
        .await;

    let bob = Repo::build_tokio()
        .with_peer_id(PeerId::from("bob"))
        .with_concurrency(ConcurrencyConfig::Threadpool(
            rayon::ThreadPoolBuilder::new().build().unwrap(),
        ))
        .load()
        .await;

    let _connected = tincans::connect_repos(&alice, &bob).await;

    let alice_handle = alice.create(Automerge::new()).await.unwrap();
    alice_handle.with_document(|am| {
        use automerge::{AutomergeError, ROOT};

        am.transact::<_, _, AutomergeError>(|tx| {
            use automerge::transaction::Transactable;

            tx.put(ROOT, "foo", "bar")?;
            Ok(())
        })
        .unwrap();
    });

    let bob_handle = bob.find(alice_handle.document_id().clone()).await.unwrap();
    assert!(bob_handle.is_some());
    bob.stop().await;
    alice.stop().await;
}

#[tokio::test]
async fn non_announcing_peers_dont_sync() {
    init_logging();

    let alice = Repo::build_tokio()
        .with_peer_id(PeerId::from("alice"))
        .with_announce_policy(|_doc_id, _peer_id| false)
        .load()
        .await;

    let bob = Repo::build_tokio()
        .with_peer_id(PeerId::from("bob"))
        .load()
        .await;

    let connected = tincans::connect_repos(&alice, &bob).await;

    let alice_handle = alice.create(Automerge::new()).await.unwrap();
    alice_handle.with_document(|am| {
        use automerge::{AutomergeError, ROOT};

        am.transact::<_, _, AutomergeError>(|tx| {
            use automerge::transaction::Transactable;

            tx.put(ROOT, "foo", "bar")?;
            Ok(())
        })
        .unwrap();
    });

    // Give alice time to have published the document changes (she shouldn't
    // publish changes because of the announce policy, but if she does due to a
    // bug we need to wait for that to happen)
    tokio::time::sleep(Duration::from_millis(100)).await;

    connected.disconnect().await;

    // Bob should not find the document because alice did not announce it
    let bob_handle = bob.find(alice_handle.document_id().clone()).await.unwrap();
    assert!(bob_handle.is_none());
    bob.stop().await;
    alice.stop().await;
}

#[cfg(feature = "tokio")]
#[tokio::test]
async fn ephemera_smoke() {
    use std::sync::{Arc, Mutex};

    init_logging();

    let alice = Repo::build_tokio()
        .with_peer_id(PeerId::from("alice"))
        .load()
        .await;

    let bob = Repo::build_tokio()
        .with_peer_id(PeerId::from("bob"))
        .load()
        .await;

    let _connected = tincans::connect_repos(&alice, &bob).await;

    let alice_handle = alice.create(Automerge::new()).await.unwrap();
    let bob_handle = bob
        .find(alice_handle.document_id().clone())
        .await
        .unwrap()
        .unwrap();

    let bob_received = Arc::new(Mutex::new(Vec::new()));

    tokio::spawn({
        let bob_received = bob_received.clone();
        async move {
            use tokio_stream::StreamExt;

            let mut ephemeral = bob_handle.ephemera();
            while let Some(msg) = ephemeral.next().await {
                bob_received.lock().unwrap().push(msg);
            }
        }
    });

    alice_handle.broadcast(vec![1, 2, 3]);

    tokio::time::sleep(Duration::from_millis(100)).await;

    assert_eq!(*bob_received.lock().unwrap(), vec![vec![1, 2, 3]]);
    bob.stop().await;
    alice.stop().await;
}

#[cfg(feature = "tokio")]
#[tokio::test]
async fn change_listeners_smoke() {
    use std::sync::{Arc, Mutex};
    init_logging();

    let alice = Repo::build_tokio()
        .with_peer_id(PeerId::from("alice"))
        .load()
        .await;

    let bob = Repo::build_tokio()
        .with_peer_id(PeerId::from("bob"))
        .load()
        .await;

    let _connected = tincans::connect_repos(&alice, &bob).await;

    let alice_handle = alice.create(Automerge::new()).await.unwrap();
    let bob_handle = bob
        .find(alice_handle.document_id().clone())
        .await
        .unwrap()
        .unwrap();

    let bob_received = Arc::new(Mutex::new(Vec::new()));

    tokio::spawn({
        let bob_received = bob_received.clone();
        async move {
            use tokio_stream::StreamExt;

            let mut changes = bob_handle.changes();
            while let Some(change) = changes.next().await {
                bob_received.lock().unwrap().push(change.new_heads);
            }
        }
    });

    let new_heads = alice_handle.with_document(|doc| {
        use automerge::{AutomergeError, ROOT};

        doc.transact::<_, _, AutomergeError>(|tx| {
            use automerge::transaction::Transactable;

            tx.put(ROOT, "foo", "bar")?;
            Ok(())
        })
        .unwrap();
        doc.get_heads()
    });

    tokio::time::sleep(Duration::from_millis(100)).await;

    assert_eq!(*bob_received.lock().unwrap(), vec![new_heads]);
    bob.stop().await;
    alice.stop().await;
}

#[cfg(feature = "tokio")]
#[tokio::test]
async fn peer_state_listeners_smoke() {
    use std::sync::{Arc, Mutex};
    init_logging();

    let alice = Repo::build_tokio()
        .with_peer_id(PeerId::from("alice"))
        .load()
        .await;

    let bob = Repo::build_tokio()
        .with_peer_id(PeerId::from("bob"))
        .load()
        .await;

    // alice dials bob (left=alice, right=bob)
    let connected = tincans::connect_repos(&alice, &bob).await;

    let alice_handle = alice.create(Automerge::new()).await.unwrap();
    let bob_handle = bob
        .find(alice_handle.document_id().clone())
        .await
        .unwrap()
        .unwrap();

    let (peer_states_on_bob, mut more_states) = bob_handle.peers();
    // Bob has one connection (alice connected via acceptor)
    assert_eq!(peer_states_on_bob.len(), 1);

    // Bob sees alice via the right_connection_id
    let bob_conn_to_alice = connected.right_connection_id;
    assert!(peer_states_on_bob.contains_key(&bob_conn_to_alice));

    // Now make a change on alice

    let bob_received = Arc::new(Mutex::new(Vec::new()));

    tokio::spawn({
        let bob_received = bob_received.clone();
        async move {
            use tokio_stream::StreamExt;

            while let Some(change) = more_states.next().await {
                bob_received
                    .lock()
                    .unwrap()
                    .push(change.get(&bob_conn_to_alice).unwrap().shared_heads.clone());
            }
        }
    });

    let new_heads = alice_handle.with_document(|doc| {
        use automerge::{AutomergeError, ROOT};

        doc.transact::<_, _, AutomergeError>(|tx| {
            use automerge::transaction::Transactable;

            tx.put(ROOT, "foo", "bar")?;
            Ok(())
        })
        .unwrap();
        doc.get_heads()
    });

    tokio::time::sleep(Duration::from_millis(100)).await;

    assert_eq!(
        *bob_received.lock().unwrap().last().unwrap(),
        Some(new_heads),
    );
}

#[cfg(feature = "tokio")]
#[tokio::test]
async fn they_have_our_changes_smoke() {
    use std::sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    };

    init_logging();

    let alice = Repo::build_tokio()
        .with_peer_id(PeerId::from("alice"))
        .load()
        .await;

    let bob = Repo::build_tokio()
        .with_peer_id(PeerId::from("bob"))
        .with_announce_policy(|_, _| false)
        .load()
        .await;

    // now create a doc on bob
    let mut doc = Automerge::new();
    doc.transact(|tx| {
        use automerge::{ROOT, transaction::Transactable};

        tx.put(ROOT, "foo", "bar")?;
        Ok::<(), automerge::AutomergeError>(())
    })
    .unwrap();
    let bob_handle = bob.create(doc).await.unwrap();

    let connected = tincans::connect_repos(&bob, &alice).await;
    // bob's connection to alice
    let bob_conn_to_alice = connected.left_connection_id;

    let alice_has_changes = Arc::new(AtomicBool::new(false));

    // Spawn a task which will update the flag
    tokio::spawn({
        let alice_has_changes = alice_has_changes.clone();
        let bob_handle = bob_handle.clone();
        async move {
            use std::sync::atomic::Ordering;

            bob_handle.they_have_our_changes(bob_conn_to_alice).await;
            alice_has_changes.store(true, Ordering::SeqCst);
        }
    });

    // Now wait 100 millis
    tokio::time::sleep(Duration::from_millis(100)).await;

    // alice_has_changes should not have been flipped because bob doesn't announce
    assert!(!alice_has_changes.load(Ordering::SeqCst));

    // Now find the document on alice, which will trigger sync with bob

    // Check that alice has the same changes
    let _alice_handle = alice
        .find(bob_handle.document_id().clone())
        .await
        .unwrap()
        .unwrap();

    // The flag should have been flipped
    assert!(alice_has_changes.load(Ordering::SeqCst));
}

#[cfg(feature = "tokio")]
#[tokio::test]
async fn connected_peers_smoke() {
    use std::sync::{Arc, Mutex};

    use samod::ConnectionState;

    init_logging();

    let alice = Repo::build_tokio()
        .with_peer_id(PeerId::from("alice"))
        .load()
        .await;

    let bob = Repo::build_tokio()
        .with_peer_id(PeerId::from("bob"))
        .load()
        .await;

    let (init_connected, mut conn_event_stream) = alice.connected_peers();
    assert!(init_connected.is_empty());

    let conn_events = Arc::new(Mutex::new(Vec::new()));
    tokio::spawn({
        let conn_events = conn_events.clone();
        async move {
            use futures::StreamExt;

            while let Some(infos) = conn_event_stream.next().await {
                conn_events.lock().unwrap().push(infos);
            }
        }
    });

    // Connect the two peers (alice dials bob)
    let connected = tincans::connect_repos(&alice, &bob).await;
    let alice_conn_id = connected.left_connection_id;

    // Give the connection event stream a moment to process
    tokio::time::sleep(Duration::from_millis(50)).await;

    // Now we should have a bunch of connection events recorded
    assert!(!conn_events.lock().unwrap().is_empty());

    // The first event should be the existence of a new handshaking connection
    let first = conn_events.lock().unwrap().first().unwrap().clone();
    let bob_info = first.iter().find(|info| info.id == alice_conn_id).unwrap();
    assert_eq!(bob_info.state, ConnectionState::Handshaking);

    // The next event should be the completion of the handshake
    let second = conn_events.lock().unwrap().get(1).unwrap().clone();
    let bob_info = second.iter().find(|info| info.id == alice_conn_id).unwrap();
    assert_eq!(
        bob_info.state,
        ConnectionState::Connected {
            their_peer_id: bob.peer_id().clone()
        }
    );

    // Now create a document on Alice and ensure that Bob can find it
    let alice_handle = alice.create(Automerge::new()).await.unwrap();

    let bob_handle = bob
        .find(alice_handle.document_id().clone())
        .await
        .unwrap()
        .unwrap();

    // Bob's connection to alice
    let bob_conn_to_alice = connected.right_connection_id;
    bob_handle.we_have_their_changes(bob_conn_to_alice).await;

    // Now get the last event received
    let last = conn_events.lock().unwrap().last().unwrap().clone();
    let bob_info = last.iter().find(|info| info.id == alice_conn_id).unwrap();
    let doc_info = bob_info.docs.get(bob_handle.document_id()).unwrap();
    assert_eq!(
        doc_info.their_heads,
        Some(alice_handle.with_document(|d| d.get_heads()))
    );
}

#[cfg(feature = "tokio")]
#[tokio::test]
async fn when_connected_resolves_after_connection() {
    init_logging();

    let alice = Repo::build_tokio()
        .with_peer_id(PeerId::from("alice"))
        .load()
        .await;

    let bob = Repo::build_tokio()
        .with_peer_id(PeerId::from("bob"))
        .load()
        .await;

    let bob_peer_id = bob.peer_id();

    // Start waiting for connection before it exists
    let when_connected_fut = alice.when_connected(bob_peer_id.clone());

    // Now connect the two repos (alice dials bob)
    let _connected = tincans::connect_repos(&alice, &bob).await;

    // The when_connected future should resolve
    let conn = tokio::time::timeout(Duration::from_secs(5), when_connected_fut)
        .await
        .expect("when_connected timed out")
        .expect("when_connected returned Stopped");

    // The connection should report bob's peer id
    let info = conn.info().expect("connection should have peer info");
    assert_eq!(info.peer_id, bob_peer_id);

    alice.stop().await;
    bob.stop().await;
}

#[cfg(feature = "tokio")]
#[tokio::test]
async fn when_connected_resolves_immediately_if_already_connected() {
    init_logging();

    let alice = Repo::build_tokio()
        .with_peer_id(PeerId::from("alice"))
        .load()
        .await;

    let bob = Repo::build_tokio()
        .with_peer_id(PeerId::from("bob"))
        .load()
        .await;

    let bob_peer_id = bob.peer_id();

    // Connect first
    let _connected = tincans::connect_repos(&alice, &bob).await;

    // Now call when_connected — should resolve immediately since bob is already connected
    let conn = tokio::time::timeout(
        Duration::from_secs(1),
        alice.when_connected(bob_peer_id.clone()),
    )
    .await
    .expect("when_connected timed out (should have been immediate)")
    .expect("when_connected returned Stopped");

    let info = conn.info().expect("connection should have peer info");
    assert_eq!(info.peer_id, bob_peer_id);

    alice.stop().await;
    bob.stop().await;
}

#[cfg(feature = "tokio")]
#[tokio::test]
async fn when_connected_returns_correct_connection() {
    init_logging();

    let alice = Repo::build_tokio()
        .with_peer_id(PeerId::from("alice"))
        .load()
        .await;

    let bob = Repo::build_tokio()
        .with_peer_id(PeerId::from("bob"))
        .load()
        .await;

    let carol = Repo::build_tokio()
        .with_peer_id(PeerId::from("carol"))
        .load()
        .await;

    let bob_peer_id = bob.peer_id();
    let carol_peer_id = carol.peer_id();

    // Connect alice to both bob and carol
    let _connected_bob = tincans::connect_repos(&alice, &bob).await;
    let _connected_carol = tincans::connect_repos(&alice, &carol).await;

    // when_connected should resolve to the correct peer in each case
    let bob_conn = tokio::time::timeout(
        Duration::from_secs(1),
        alice.when_connected(bob_peer_id.clone()),
    )
    .await
    .expect("when_connected(bob) timed out")
    .expect("when_connected(bob) returned Stopped");

    let carol_conn = tokio::time::timeout(
        Duration::from_secs(1),
        alice.when_connected(carol_peer_id.clone()),
    )
    .await
    .expect("when_connected(carol) timed out")
    .expect("when_connected(carol) returned Stopped");

    assert_eq!(bob_conn.info().unwrap().peer_id, bob_peer_id);
    assert_eq!(carol_conn.info().unwrap().peer_id, carol_peer_id);
    // Different connections to different peers
    assert_ne!(bob_conn.id(), carol_conn.id());

    alice.stop().await;
    bob.stop().await;
    carol.stop().await;
}

#[cfg(feature = "tokio")]
#[tokio::test]
async fn when_connected_multiple_waiters_same_peer() {
    init_logging();

    let alice = Repo::build_tokio()
        .with_peer_id(PeerId::from("alice"))
        .load()
        .await;

    let bob = Repo::build_tokio()
        .with_peer_id(PeerId::from("bob"))
        .load()
        .await;

    let bob_peer_id = bob.peer_id();

    // Multiple tasks wait for the same peer
    let fut1 = alice.when_connected(bob_peer_id.clone());
    let fut2 = alice.when_connected(bob_peer_id.clone());

    // Now connect
    let _connected = tincans::connect_repos(&alice, &bob).await;

    // Both should resolve
    let conn1 = tokio::time::timeout(Duration::from_secs(5), fut1)
        .await
        .expect("when_connected #1 timed out")
        .expect("when_connected #1 returned Stopped");

    let conn2 = tokio::time::timeout(Duration::from_secs(5), fut2)
        .await
        .expect("when_connected #2 timed out")
        .expect("when_connected #2 returned Stopped");

    assert_eq!(conn1.info().unwrap().peer_id, bob_peer_id);
    assert_eq!(conn2.info().unwrap().peer_id, bob_peer_id);
    // Both should be the same connection
    assert_eq!(conn1.id(), conn2.id());

    alice.stop().await;
    bob.stop().await;
}
