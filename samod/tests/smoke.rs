#![cfg(feature = "tokio")]

use std::time::Duration;

use automerge::Automerge;
use samod::{ConcurrencyConfig, PeerId, Repo, storage::InMemoryStorage};
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
    use samod::PeerId;

    init_logging();

    let alice = Repo::build_tokio()
        .with_peer_id(PeerId::from("alice"))
        .load()
        .await;

    let bob = Repo::build_tokio()
        .with_peer_id(PeerId::from("bob"))
        .load()
        .await;

    tincans::connect_repos(&alice, &bob);

    bob.when_connected(alice.peer_id()).await.unwrap();
    alice.when_connected(bob.peer_id()).await.unwrap();

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
    use samod::PeerId;

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

    tincans::connect_repos(&alice, &bob);

    bob.when_connected(alice.peer_id()).await.unwrap();
    alice.when_connected(bob.peer_id()).await.unwrap();

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

    let connection = tincans::connect_repos(&alice, &bob);

    bob.when_connected(alice.peer_id()).await.unwrap();
    alice.when_connected(bob.peer_id()).await.unwrap();

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

    connection.disconnect().await;

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

    let _connection = tincans::connect_repos(&alice, &bob);

    bob.when_connected(alice.peer_id()).await.unwrap();
    alice.when_connected(bob.peer_id()).await.unwrap();

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

    let _connection = tincans::connect_repos(&alice, &bob);

    bob.when_connected(alice.peer_id()).await.unwrap();
    alice.when_connected(bob.peer_id()).await.unwrap();

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
