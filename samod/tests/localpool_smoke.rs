use std::sync::Arc;

use automerge::{
    Automerge, AutomergeError, ROOT, ReadDoc, ScalarValue, Value, transaction::Transactable,
};
use futures::{executor::LocalPool, task::LocalSpawnExt};
use samod::{BackoffConfig, ConcurrencyConfig, transport::channel::ChannelDialer};
use url::Url;

fn init_logging() {
    let _ = tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .try_init();
}

#[test]
fn test_localpool() {
    init_logging();

    std::thread::spawn(|| {
        let mut pool = LocalPool::new();
        let spawner = pool.spawner();

        pool.spawner()
            .spawn_local(async move {
                let alice = samod::Repo::build_localpool(spawner.clone())
                    .with_peer_id("alice".into())
                    .load_local()
                    .await;

                let bob = samod::Repo::build_localpool(spawner.clone())
                    .with_peer_id("bob".into())
                    .with_concurrency(ConcurrencyConfig::AsyncRuntime)
                    .load_local()
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

                // Bob sets up an acceptor, alice dials bob
                let url = Url::parse("ws://test-localpool:0").unwrap();
                let acceptor = bob.make_acceptor(url.clone()).unwrap();

                let dialer = ChannelDialer::new(acceptor);

                let dialer_handle = alice
                    .dial(BackoffConfig::default(), Arc::new(dialer))
                    .unwrap();

                // Wait for the connection to be ready
                dialer_handle.established().await.unwrap();

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
