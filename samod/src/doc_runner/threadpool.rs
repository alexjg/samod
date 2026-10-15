use std::{
    collections::VecDeque,
    sync::{Arc, Mutex},
};

use crate::{actor_task::ActorTask, doc_actor_inner::DocActorInner};

/// A document's FIFO mailbox, shared by hub/IO dispatch and DocHandle operations.
/// Only one runner drains it, so Rayon scheduling cannot reorder actor tasks.
/// Idle documents do not occupy a worker thread.
pub(crate) struct ThreadpoolActor {
    inner: Arc<Mutex<DocActorInner>>,
    pool: Arc<rayon::ThreadPool>,
    mailbox: Mutex<Mailbox>,
}

#[derive(Default)]
struct Mailbox {
    tasks: VecDeque<ActorTask>,
    running: bool,
}

impl ThreadpoolActor {
    pub(crate) fn new(inner: Arc<Mutex<DocActorInner>>, pool: Arc<rayon::ThreadPool>) -> Arc<Self> {
        Arc::new(Self {
            inner,
            pool,
            mailbox: Mutex::new(Mailbox::default()),
        })
    }

    pub(crate) fn dispatch(self: &Arc<Self>, task: ActorTask) {
        let mut mailbox = self.mailbox.lock().unwrap();
        mailbox.tasks.push_back(task);
        if mailbox.running {
            return;
        }
        mailbox.running = true;
        drop(mailbox);
        self.schedule();
    }

    fn schedule(self: &Arc<Self>) {
        let actor = self.clone();
        self.pool.spawn_fifo(move || actor.run());
    }

    fn run(self: Arc<Self>) {
        // Yield between batches so a busy document does not monopolize the pool.
        // spawn_fifo also avoids putting this actor ahead of other queued actors
        // when rescheduling from within a Rayon worker.
        const BATCH_SIZE: usize = 64;
        for _ in 0..BATCH_SIZE {
            let task = {
                let mut mailbox = self.mailbox.lock().unwrap();
                let Some(task) = mailbox.tasks.pop_front() else {
                    // Empty detection and relinquishing the runner must be atomic
                    // with dispatch, otherwise an arriving task could be stranded.
                    mailbox.running = false;
                    return;
                };
                task
            };
            // Never hold the mailbox lock while executing user code or waiting
            // for synchronous DocHandle access to release the document lock.
            self.inner.lock().unwrap().handle_task(task);
        }
        self.schedule();
    }
}

#[cfg(all(test, feature = "tokio"))]
mod tests {
    use std::{sync::mpsc, time::Duration};

    use automerge::Automerge;
    use futures::{
        FutureExt, StreamExt,
        channel::{mpsc as async_mpsc, oneshot},
    };

    use super::*;
    use crate::{ConcurrencyConfig, Repo, doc_runner::DocRunner};

    async fn repo(threads: usize) -> Repo {
        Repo::build_tokio()
            .with_concurrency(ConcurrencyConfig::Threadpool(
                rayon::ThreadPoolBuilder::new()
                    .num_threads(threads)
                    .build()
                    .unwrap(),
            ))
            .load()
            .await
    }

    #[tokio::test]
    async fn hub_and_handle_tasks_share_fifo_order_even_when_submitted_from_a_worker() {
        tokio::time::timeout(Duration::from_secs(5), async {
            let repo = repo(1).await;
            let doc = repo.create(Automerge::new()).await.unwrap();
            let (pool, actor_id) = {
                let inner = repo.inner.lock().unwrap();
                let DocRunner::Threadpool(pool) = &inner.doc_runner else {
                    unreachable!()
                };
                (pool.clone(), *inner.actors.keys().next().unwrap())
            };
            // Repeat to exercise both active and idle mailbox dispatch. More than
            // one batch also checks ordering across rescheduling boundaries.
            for _ in 0..3 {
                let (tx, mut rx) = async_mpsc::unbounded();
                let inner = repo.inner.clone();
                let doc = doc.clone();
                pool.spawn(move || {
                    for index in 0..200 {
                        let tx = tx.clone();
                        if index % 2 == 0 {
                            // Poll once to submit the DocHandle task without waiting
                            // for the only worker (this one) to execute it.
                            assert!(
                                doc.with_document_async(move |_| {
                                    tx.unbounded_send(index).unwrap();
                                })
                                .now_or_never()
                                .is_none()
                            );
                        } else {
                            inner.lock().unwrap().dispatch_task(
                                actor_id,
                                ActorTask::WithDocument(Box::new(move |_| {
                                    tx.unbounded_send(index).unwrap();
                                })),
                            );
                        }
                    }
                });
                let mut actual = Vec::new();
                while let Some(index) = rx.next().await {
                    actual.push(index);
                }
                assert_eq!(actual, (0..200).collect::<Vec<_>>());
            }
            repo.stop().await;
        })
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn queued_tasks_for_a_busy_actor_do_not_occupy_other_workers() {
        tokio::time::timeout(Duration::from_secs(5), async {
            let repo = repo(2).await;
            let busy = repo.create(Automerge::new()).await.unwrap();
            let other = repo.create(Automerge::new()).await.unwrap();
            let (started, ready) = oneshot::channel();
            let (release, wait) = mpsc::channel();
            let mut blocked = Box::pin(busy.with_document_async(move |_| {
                let _ = started.send(());
                // Bound failures even if the async test times out.
                let _ = wait.recv_timeout(Duration::from_secs(5));
            }));
            assert!(futures::poll!(&mut blocked).is_pending());
            ready.await.unwrap();
            let mut queued = Vec::new();
            for _ in 0..100 {
                let mut task = Box::pin(busy.with_document_async(|_| ()));
                assert!(futures::poll!(&mut task).is_pending());
                queued.push(task);
            }
            other.with_document_async(|_| ()).await.unwrap();
            release.send(()).unwrap();
            blocked.await.unwrap();
            for task in queued {
                task.await.unwrap();
            }
            repo.stop().await;
        })
        .await
        .unwrap();
    }
}
