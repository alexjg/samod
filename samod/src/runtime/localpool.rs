use futures::{FutureExt, executor::LocalSpawner, task::LocalSpawnExt};

use crate::runtime::RuntimeHandle;

#[derive(Clone)]
pub struct LocalPoolRuntime;

impl RuntimeHandle for LocalSpawner {
    type JoinErr = SpawnError;
    type JoinFuture<O: Send + 'static> =
        futures::future::LocalBoxFuture<'static, Result<O, Self::JoinErr>>;

    fn spawn<O, F>(&self, f: F) -> Self::JoinFuture<O>
    where
        O: Send + 'static,
        F: Future<Output = O> + 'static,
    {
        let (tx, rx) = futures::channel::oneshot::channel();
        // Note we can't use `spawn_local_with_handle` because the returned `RemoteHandle`
        // cancels the underlying future when it is dropped, which is not the semantics we
        // want.
        let spawn_result = self.spawn_local(async move {
            let res = f.await;
            let _ = tx.send(res);
        });
        async move {
            if let Err(e) = spawn_result {
                return Err(SpawnError(e));
            }
            let result = rx
                .await
                .expect("the future was spawned and the last thing we do is send to the channel");
            Ok(result)
        }
        .boxed_local()
    }
}

pub struct SpawnError(futures::task::SpawnError);

impl std::fmt::Display for SpawnError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}

impl std::fmt::Debug for SpawnError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}

impl std::error::Error for SpawnError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        self.0.source()
    }
}

impl super::JoinError for SpawnError {
    fn is_panic(&self) -> bool {
        false
    }

    fn into_panic(self) -> Box<dyn std::any::Any + Send + 'static> {
        panic!("not a panic");
    }
}
