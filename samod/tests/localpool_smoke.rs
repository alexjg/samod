use std::convert::Infallible;

use automerge::{
    Automerge, AutomergeError, ROOT, ReadDoc, ScalarValue, Value, transaction::Transactable,
};
use futures::{
    FutureExt as _, StreamExt,
    executor::LocalPool,
    task::{LocalSpawnExt, SpawnExt},
};
use samod::ConnDirection;

fn init_logging() {
    let _ = tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .try_init();
}

#[test]
fn test_localpool() {
    init_logging();
    let (tx_to_bob, rx_from_alice) = futures::channel::mpsc::unbounded::<Vec<u8>>();
    let (tx_to_alice, rx_from_bob) = futures::channel::mpsc::unbounded::<Vec<u8>>();

    std::thread::spawn(|| {
        let mut pool = LocalPool::new();
        let spawner = pool.spawner();

        pool.spawner()
            .spawn_local(async move {
                let alice = samod::Repo::build_localpool(spawner.clone())
                    .with_peer_id("alice".into())
                    .with_threadpool(None)
                    .load()
                    .await;

                let bob = samod::Repo::build_localpool(spawner.clone())
                    .with_peer_id("bob".into())
                    .with_threadpool(None)
                    .load()
                    .await;

                // Create the document on alice
                let alice_handle = alice.create(Automerge::new()).await.unwrap();
                alice_handle.with_document(|doc| {
                    doc.transact(|tx| {
                        tx.put(ROOT, "foo", "bar")?;
                        Ok::<_, AutomergeError>(())
                    })
                    .unwrap();
                });

                // Connect bob and alice to each other
                let alice_conn_driver = alice
                    .connect(
                        rx_from_bob.map(Ok::<_, Infallible>),
                        tx_to_bob,
                        ConnDirection::Incoming,
                    )
                    .map(|_| ());
                spawner.spawn(alice_conn_driver).unwrap();
                let bob_conn_driver = bob
                    .connect(
                        rx_from_alice.map(Ok::<_, Infallible>),
                        tx_to_alice,
                        ConnDirection::Outgoing,
                    )
                    .map(|_| ());
                spawner.spawn(bob_conn_driver).unwrap();

                // Wait for the connection to be ready
                bob.when_connected("alice".into()).await.unwrap();

                // Lookup the doc handle on Bob
                let bob_handle = bob
                    .find(alice_handle.document_id().clone())
                    .await
                    .unwrap()
                    .expect("Bob should find Alice's document");
                tracing::info!("found the doc");

                // Verify the document content
                bob_handle.with_document(|doc| {
                    let (val, _) = doc
                        .get(ROOT, "foo")
                        .expect("Bob should read 'foo' from Alice's document")
                        .expect("Bob should find 'foo' in Alice's document");
                    let Value::Scalar(val) = val else {
                        panic!("Expected 'foo' to be a scalar value");
                    };
                    let ScalarValue::Str(s) = val.as_ref() else {
                        panic!("Expected 'foo' to be a string");
                    };
                    assert_eq!(s, &"bar");
                });

                alice.stop().await;
                bob.stop().await;
            })
            .unwrap();
        pool.run();
    });
}
