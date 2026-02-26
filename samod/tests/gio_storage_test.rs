#[cfg(feature = "gio")]
mod gio_tests {
    use tempfile::TempDir;

    use samod::storage::{testing::StorageTestFixture, GioFilesystemStorage, Storage};
    use samod_core::StorageKey;

    fn init_logging() {
        let _ = tracing_subscriber::fmt()
            .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
            .try_init();
    }

    struct GioFilesystemStorageFixture {
        storage: GioFilesystemStorage,
        _temp_dir: TempDir,
    }

    impl StorageTestFixture for GioFilesystemStorageFixture {
        type Storage = GioFilesystemStorage;

        async fn setup() -> Self {
            let temp_dir = TempDir::new().unwrap();
            let storage = GioFilesystemStorage::new(temp_dir.path());
            Self {
                storage,
                _temp_dir: temp_dir,
            }
        }

        fn storage(&self) -> &Self::Storage {
            &self.storage
        }
    }

    #[test]
    fn gio_filesystem_storage_standard_tests() {
        init_logging();

        let main_context = glib::MainContext::new();
        main_context.block_on(async {
            samod::storage::testing::run_storage_adapter_tests::<GioFilesystemStorageFixture>()
                .await;
        });
    }

    #[test]
    fn test_gio_storage_nested_directories() {
        init_logging();

        let main_context = glib::MainContext::new();
        main_context.block_on(async {
            let temp_dir = TempDir::new().unwrap();
            let storage = GioFilesystemStorage::new(temp_dir.path());

            // Create a deeply nested key
            let key =
                StorageKey::from_parts(vec!["level1", "level2", "level3", "file.txt"]).unwrap();

            let data = b"nested data".to_vec();

            // Put should create all necessary directories
            storage.put(key.clone(), data.clone()).await;

            // Verify we can load it back
            let loaded = storage.load(key).await;
            assert_eq!(loaded, Some(data));
        });
    }

    #[test]
    fn test_gio_storage_key_splaying() {
        init_logging();

        let main_context = glib::MainContext::new();
        main_context.block_on(async {
            let temp_dir = TempDir::new().unwrap();
            let storage = GioFilesystemStorage::new(temp_dir.path());

            // Test that keys are properly splayed (first component split by first two chars)
            let key = StorageKey::from_parts(vec!["abcdef", "file.txt"]).unwrap();

            let data = b"splayed data".to_vec();
            storage.put(key.clone(), data.clone()).await;

            // Verify the file structure matches the splaying logic
            let expected_path = temp_dir.path().join("ab").join("cdef").join("file.txt");

            assert!(expected_path.exists());

            // Verify we can still load it through the storage interface
            let loaded = storage.load(key).await;
            assert_eq!(loaded, Some(data));
        });
    }
}
