#![cfg(feature = "tokio")]
#![cfg(feature = "threadpool")]

use std::time::Duration;

use automerge::transaction::Transactable;
use automerge::{Automerge, ROOT, ReadDoc};
use samod::{ConcurrencyConfig, DocHandle, PeerId, Repo};

fn init_logging() {
    let _ = tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .try_init();
}

#[tokio::test]
async fn many_docs_can_be_created_and_modified() {
    init_logging();

    const NUM_THREADS: usize = 4;

    let alice = Repo::build_tokio()
        .with_concurrency(ConcurrencyConfig::Threadpool(
            rayon::ThreadPoolBuilder::new()
                .num_threads(NUM_THREADS)
                .build()
                .unwrap(),
        ))
        .with_peer_id(PeerId::from("alice"))
        .load()
        .await;

    let task = tokio::spawn(async move {
        // Create more documents than threads
        let mut docs: Vec<DocHandle> = Vec::with_capacity(NUM_THREADS + 1);
        for _ in 0..NUM_THREADS + 1 {
            let doc = alice.create(Automerge::new()).await.unwrap();
            docs.push(doc);
        }

        // Now make a change to every document, this will cause a deadlock if
        // we aren't correctly releasing threadpool threads
        for (i, doc) in docs.iter().enumerate() {
            tracing::info!("Modifying document #{i}...");
            doc.with_document(|d| {
                d.transact::<_, _, automerge::AutomergeError>(|tx| {
                    tx.put(ROOT, "index", i as i64)?;
                    Ok(())
                })
                .unwrap();
            });
        }

        // Verify all changes were applied
        for (i, doc) in docs.iter().enumerate() {
            doc.with_document(|d| {
                let value = d.get(ROOT, "index").unwrap().unwrap();
                assert_eq!(
                    value.0.to_i64(),
                    Some(i as i64),
                    "document {i} has wrong value"
                );
            });
        }

        tracing::info!(
            "All {} documents created and modified successfully!",
            NUM_THREADS + 1
        );
    });

    tokio::time::timeout(Duration::from_millis(1000), task)
        .await
        .expect("task should complete")
        .expect("task should succeed");
}
