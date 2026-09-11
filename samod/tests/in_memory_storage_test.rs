mod in_memory_tests {
    use samod::storage::{testing::StorageTestFixture, InMemoryStorage};

    fn init_logging() {
        let _ = tracing_subscriber::fmt()
            .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
            .try_init();
    }

    struct InMemoryStorageFixture {
        storage: InMemoryStorage,
    }

    impl StorageTestFixture for InMemoryStorageFixture {
        type Storage = InMemoryStorage;

        async fn setup() -> Self {
            Self {
                storage: InMemoryStorage::new(),
            }
        }

        fn storage(&self) -> &Self::Storage {
            &self.storage
        }
    }

    #[tokio::test]
    async fn in_memory_storage_standard_tests() {
        init_logging();
        samod::storage::testing::run_storage_adapter_tests::<InMemoryStorageFixture>().await;
    }
}
