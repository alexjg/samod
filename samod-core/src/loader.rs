use std::sync::{Arc, Mutex};

use crate::{
    PeerId, UnixTimestamp,
    actors::{
        driver::{Driver, StepResult},
        hub::Hub,
        loading::{self, Loading},
    },
    io::{IoResult, IoTask, StorageResult, StorageTask},
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
///
/// let mut loader = SamodLoader::new(rand::rng(), PeerId::from("test"), UnixTimestamp::now());
///
/// loop {
///     match loader.step(UnixTimestamp::now()) {
///         LoaderState::NeedIo(tasks) => {
///             // Execute IO tasks and provide results
///             for task in tasks {
///                 // ... execute task ...
///                 # let result: IoResult<StorageResult> = todo!();
///                 loader.provide_io_result(UnixTimestamp::now(), result);
///             }
///         }
///         LoaderState::Loaded(samod) => {
///             // Repository is loaded and ready to use
///             break;
///         }
///     }
/// }
/// ```
pub struct SamodLoader<R> {
    driver: Driver<Loading<R>>,
}

/// The current state of the loader.
pub enum LoaderState {
    /// The loader needs IO operations to be performed.
    ///
    /// The caller should execute all provided IO tasks and call
    /// `provide_io_result` for each completed task, then call `step` again.
    NeedIo(Vec<IoTask<StorageTask>>),

    /// Loading is complete and the samod repository is ready to use.
    Loaded(Hub),
}

impl<R: rand::Rng + Clone + Send + Sync + 'static> SamodLoader<R> {
    /// Creates a new samod loader.
    ///
    /// # Arguments
    ///
    /// * `now` - The current timestamp for initialization
    ///
    /// # Returns
    ///
    /// A new `SamodLoader` ready to begin the loading process.
    pub fn new(rng: R, local_peer_id: PeerId, now: UnixTimestamp) -> Self {
        let driver = Driver::spawn(now, |args| loading::load(rng, local_peer_id, args));

        Self { driver }
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
    pub fn step(&mut self, now: UnixTimestamp) -> LoaderState {
        let new_tasks = match self.driver.step(now) {
            StepResult::Suspend(tasks) => tasks,
            StepResult::Complete {
                results,
                complete: (hub_state, rng),
            } => {
                assert!(results.is_empty());
                let state = Arc::new(Mutex::new(hub_state));
                let hub = Hub::new(rng, now, state);
                return LoaderState::Loaded(hub);
            }
        };

        LoaderState::NeedIo(new_tasks)
    }

    /// Provides the result of an IO operation.
    ///
    /// This method should be called for each IO task that was returned by `step`.
    /// The loader passes the result directly to the driver for processing.
    ///
    /// # Arguments
    ///
    /// * `result` - The result of executing an IO task
    pub fn provide_io_result(&mut self, now: UnixTimestamp, result: IoResult<StorageResult>) {
        self.driver.handle_io_complete(now, result);
    }
}
