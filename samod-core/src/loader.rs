use crate::{
    PeerId, StorageId, StorageKey, UnixTimestamp,
    actors::hub::{Hub, State as HubState},
    ephemera::EphemeralSession,
    io::{IoResult, IoTask, IoTaskId, StorageResult, StorageTask},
};

/// A state machine for loading a samod repository.
///
/// `SamodLoader` handles the initialization phase of a samod repository,
/// coordinating between the user and the driver to load or generate the storage ID
/// and perform any other setup operations required before the repository can be used.
///
/// ## Usage
///
/// ```rust,no_run
/// use samod_core::{PeerId, SamodLoader, LoaderState, UnixTimestamp, io::{StorageResult, IoResult}};
#[cfg_attr(feature = "subduction", doc = " use ed25519_dalek;")]
/// use rand::SeedableRng;
///
/// let mut rng = rand::rngs::StdRng::from_rng(&mut rand::rng());
#[cfg_attr(
    not(feature = "subduction"),
    doc = "let mut loader = SamodLoader::new(PeerId::from(\"test\"));"
)]
#[cfg_attr(
    feature = "subduction",
    doc = "let verifying_key = ed25519_dalek::SigningKey::from_bytes(&[0u8; 32]).verifying_key();"
)]
#[cfg_attr(
    feature = "subduction",
    doc = "let mut loader = SamodLoader::new(&verifying_key, None);"
)]
///
/// loop {
///     match loader.step(&mut rng, UnixTimestamp::now()) {
///         LoaderState::NeedIo(tasks) => {
///             // Execute IO tasks and provide results
///             for task in tasks {
///                 // ... execute task ...
///                 # let result: IoResult<StorageResult> = todo!();
///                 loader.provide_io_result(result);
///             }
///         }
///         LoaderState::Loaded(samod) => {
///             // Repository is loaded and ready to use
///             break;
///         }
///     }
/// }
/// ```
pub struct SamodLoader {
    local_peer_id: PeerId,
    state: State,
    #[cfg(feature = "subduction")]
    subduction_config: crate::actors::hub::subduction_sync::SubductionConfig,
}

/// The current state of the loader.
pub enum LoaderState {
    /// The loader needs IO operations to be performed.
    ///
    /// The caller should execute all provided IO tasks and call
    /// `provide_io_result` for each completed task, then call `step` again.
    NeedIo(Vec<IoTask<StorageTask>>),

    /// Loading is complete and the samod repository is ready to use.
    Loaded(Box<Hub>),
}

enum State {
    Starting,
    LoadingStorageId(IoTaskId),
    StorageIdLoaded(Option<Vec<u8>>),
    PuttingStorageId(IoTaskId, StorageId),
    Done(StorageId),
}

impl SamodLoader {
    /// Creates a new samod loader.
    ///
    /// Without the `subduction` feature, pass a [`PeerId`] directly.
    /// With the `subduction` feature, pass an [`ed25519_dalek::VerifyingKey`]
    /// — the [`PeerId`] will be derived from it.
    #[cfg(not(feature = "subduction"))]
    pub fn new(local_peer_id: PeerId) -> Self {
        Self {
            local_peer_id,
            state: State::Starting,
        }
    }

    /// Creates a new samod loader from an Ed25519 verifying key.
    ///
    /// The [`PeerId`] is derived from the verifying key, ensuring the samod
    /// and subduction peer identities match. The corresponding signing key
    /// is held by the runtime layer (not samod-core) — the hub only needs
    /// the verifying key to construct signature payloads.
    #[cfg(feature = "subduction")]
    pub fn new(
        verifying_key: &ed25519_dalek::VerifyingKey,
        responder_config: Option<crate::actors::hub::subduction_sync::ResponderConfig>,
    ) -> Self {
        let local_peer_id = PeerId::from_verifying_key(verifying_key);
        let subduction_config = crate::actors::hub::subduction_sync::SubductionConfig {
            our_verifying_key: *verifying_key,
            responder_config,
        };
        Self {
            local_peer_id,
            state: State::Starting,
            subduction_config,
        }
    }

    /// Advances the loader state machine.
    ///
    /// This method should be called repeatedly until `LoaderState::Loaded` is returned.
    /// When `LoaderState::NeedIo` is returned, the caller must execute the provided
    /// IO tasks and call `provide_io_result` for each one before calling `step` again.
    ///
    /// # Arguments
    ///
    /// * `now` - The current timestamp
    ///
    /// # Returns
    ///
    /// The current state of the loader.
    pub fn step<R: rand::Rng>(&mut self, rng: &mut R, _now: UnixTimestamp) -> LoaderState {
        match &self.state {
            State::Starting => {
                let task = IoTask::new(StorageTask::Load {
                    key: StorageKey::storage_id_path(),
                });
                self.state = State::LoadingStorageId(task.task_id);
                LoaderState::NeedIo(vec![task])
            }
            State::LoadingStorageId(_task_id) => LoaderState::NeedIo(Vec::new()),
            State::StorageIdLoaded(result) => {
                if let Some(result) = result {
                    match String::from_utf8(result.to_vec()) {
                        Ok(s) => {
                            let storage_id = StorageId::from(s);
                            self.state = State::Done(storage_id.clone());
                            let state = HubState::new(
                                storage_id,
                                self.local_peer_id.clone(),
                                EphemeralSession::new(rng),
                                #[cfg(feature = "subduction")]
                                self.subduction_config.clone(),
                            );
                            return LoaderState::Loaded(Box::new(Hub::new(state)));
                        }
                        Err(_e) => {
                            tracing::warn!("storage ID was not a valid string, creating a new one");
                        }
                    }
                } else {
                    tracing::info!("no storage ID found, generating a new one");
                }
                let storage_id = StorageId::new(rng);
                let task = IoTask::new(StorageTask::Put {
                    key: StorageKey::storage_id_path(),
                    value: storage_id.as_str().as_bytes().to_vec(),
                });
                self.state = State::PuttingStorageId(task.task_id, storage_id);
                LoaderState::NeedIo(vec![task])
            }
            State::PuttingStorageId(_task_id, _storage_id) => LoaderState::NeedIo(Vec::new()),
            State::Done(storage_id) => {
                let state = HubState::new(
                    storage_id.clone(),
                    self.local_peer_id.clone(),
                    EphemeralSession::new(rng),
                    #[cfg(feature = "subduction")]
                    self.subduction_config.clone(),
                );
                LoaderState::Loaded(Box::new(Hub::new(state)))
            }
        }
    }

    /// Provides the result of an IO operation.
    ///
    /// This method should be called for each IO task that was returned by `step`.
    /// The loader passes the result directly to the driver for processing.
    ///
    /// # Arguments
    ///
    /// * `result` - The result of executing an IO task
    pub fn provide_io_result(&mut self, result: IoResult<StorageResult>) {
        match self.state {
            State::Starting | State::Done(_) | State::StorageIdLoaded(_) => {
                panic!("unexpected IO completion");
            }
            State::LoadingStorageId(io_task_id) => {
                if io_task_id != result.task_id {
                    panic!(
                        "unexpected task ID: expected {:?}, got {:?}",
                        io_task_id, result.task_id
                    );
                }
                match result.payload {
                    StorageResult::Load { value } => {
                        self.state = State::StorageIdLoaded(value);
                    }
                    _ => panic!("unexpected storage result when loading storage ID"),
                }
            }
            State::PuttingStorageId(io_task_id, ref storage_id) => {
                if io_task_id != result.task_id {
                    panic!(
                        "unexpected task ID: expected {:?}, got {:?}",
                        io_task_id, result.task_id
                    );
                }
                match result.payload {
                    StorageResult::Put => self.state = State::Done(storage_id.clone()),
                    _ => panic!("unexpected storage result when putting storage ID"),
                }
            }
        }
    }
}
