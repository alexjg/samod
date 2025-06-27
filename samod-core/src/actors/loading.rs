use std::marker::PhantomData;

use crate::{
    PeerId, StorageId, StorageKey,
    ephemera::EphemeralSession,
    io::{IoTask, StorageResult, StorageTask},
};

use super::{
    driver::{self, Actor, ActorIo},
    hub::{self},
};

pub(crate) struct Loading<R>(PhantomData<R>);

impl<R> Actor for Loading<R> {
    type IoTaskAction = StorageTask;
    type IoResult = StorageResult;
    type StepResults = Vec<IoTask<StorageTask>>;
    type Output = ();
    type Input = ();
    type Complete = (hub::State, R);

    fn finish_step(
        _outputs: Vec<Self::Output>,
        new_io_tasks: Vec<IoTask<StorageTask>>,
    ) -> Self::StepResults {
        new_io_tasks
    }
}

pub(crate) async fn load<R: rand::Rng + Clone + 'static>(
    mut rng: R,
    local_peer_id: PeerId,
    args: driver::SpawnArgs<Loading<R>>,
) -> (hub::State, R) {
    let storage_id = load_or_create_storage_id(IoAccess(args.io), rng.clone()).await;
    let ephemeral_session = EphemeralSession::new(&mut rng);
    tracing::info!(%storage_id, "storage ID loaded/generated");
    (
        hub::State::new(storage_id, local_peer_id, ephemeral_session),
        rng,
    )
}

async fn load_or_create_storage_id<R: rand::Rng + Clone + 'static>(
    io: IoAccess<R>,
    mut rng: R,
) -> StorageId {
    let storage_key = StorageKey::storage_id_path();

    let result = io.load(storage_key.clone()).await;
    if let Some(stored_bytes) = result {
        match String::from_utf8(stored_bytes) {
            Ok(s) => {
                tracing::debug!(%s, "loaded storage id");
                return StorageId::from(s);
            }
            Err(e) => {
                tracing::error!(%e, "storage Id was not a valid UTF-8 string");
            }
        }
    }

    tracing::debug!("no valid storage id found, generating a new one");

    // Generate new storage ID
    let new_storage_id = StorageId::new(&mut rng);

    // Store the new storage ID
    io.put(storage_key, new_storage_id.as_str().as_bytes().to_vec())
        .await;

    new_storage_id
}

struct IoAccess<R>(ActorIo<Loading<R>>);

impl<R> IoAccess<R> {
    async fn load(&self, key: StorageKey) -> Option<Vec<u8>> {
        let task = StorageTask::Load { key };
        let result = self.0.perform_io(task).await;
        match result {
            StorageResult::Load { value } => value,
            _ => panic!("unexpected storage result for load request"),
        }
    }

    async fn put(&self, key: StorageKey, data: Vec<u8>) {
        let task = StorageTask::Put { key, value: data };
        let result = self.0.perform_io(task).await;
        match result {
            StorageResult::Put => (),
            _ => panic!("unexpected storage result for put request"),
        }
    }
}
