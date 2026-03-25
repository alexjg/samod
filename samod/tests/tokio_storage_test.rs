#[cfg(feature = "tokio")]
mod tokio_tests {
    use tempfile::TempDir;

    use samod::storage::{testing::StorageTestFixture, Storage, TokioFilesystemStorage};
    use samod_core::StorageKey;

    fn init_logging() {
        let _ = tracing_subscriber::fmt()
            .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
            .try_init();
    }

    struct TokioFilesystemStorageFixture {
        storage: TokioFilesystemStorage,
        _temp_dir: TempDir,
    }

    impl StorageTestFixture for TokioFilesystemStorageFixture {
        type Storage = TokioFilesystemStorage;

        async fn setup() -> Self {
            let temp_dir = TempDir::new().unwrap();
            let storage = TokioFilesystemStorage::new(temp_dir.path());
            Self {
                storage,
                _temp_dir: temp_dir,
            }
        }

        fn storage(&self) -> &Self::Storage {
            &self.storage
        }
    }

    #[tokio::test]
    async fn tokio_filesystem_storage_standard_tests() {
        init_logging();
        samod::storage::testing::run_storage_adapter_tests::<TokioFilesystemStorageFixture>().await;
    }

    #[tokio::test]
    async fn test_tokio_storage_nested_directories() {
        init_logging();

        let temp_dir = TempDir::new().unwrap();
        let storage = TokioFilesystemStorage::new(temp_dir.path());

        // Create a deeply nested key
        let key = StorageKey::from_parts(vec!["level1", "level2", "level3", "file.txt"]).unwrap();

        let data = b"nested data".to_vec();

        // Put should create all necessary directories
        storage.put(key.clone(), data.clone()).await;

        // Verify we can load it back
        let loaded = storage.load(key).await;
        assert_eq!(loaded, Some(data));
    }

    #[tokio::test]
    async fn test_tokio_storage_key_splaying() {
        init_logging();

        let temp_dir = TempDir::new().unwrap();
        let storage = TokioFilesystemStorage::new(temp_dir.path());

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
    }

    #[tokio::test]
    async fn test_tokio_storage_concurrent_operations() {
        init_logging();

        let temp_dir = TempDir::new().unwrap();
        let storage = TokioFilesystemStorage::new(temp_dir.path());

        // Create multiple concurrent put operations
        let mut handles = Vec::new();
        for i in 0..10 {
            let storage = storage.clone();
            let key = StorageKey::from_parts(vec![format!("concurrent_{}", i)]).unwrap();
            let data = format!("data_{i}").into_bytes();

            let handle = tokio::spawn(async move {
                storage.put(key.clone(), data.clone()).await;
                let loaded = storage.load(key).await;
                assert_eq!(loaded, Some(data));
            });
            handles.push(handle);
        }

        // Wait for all operations to complete
        for handle in handles {
            handle.await.unwrap();
        }
    }

    #[tokio::test]
    async fn test_tokio_storage_special_characters() {
        init_logging();

        let temp_dir = TempDir::new().unwrap();
        let storage = TokioFilesystemStorage::new(temp_dir.path());

        // Test with various special characters that should be valid in filenames
        let test_cases = vec![
            "file_with_underscores",
            "file-with-dashes",
            "file.with.dots",
            "file with spaces",
            "file123numbers",
        ];

        for filename in test_cases {
            let key = StorageKey::from_parts(vec![filename]).unwrap();
            let data = format!("data for {filename}").into_bytes();

            storage.put(key.clone(), data.clone()).await;
            let loaded = storage.load(key).await;
            assert_eq!(loaded, Some(data), "Failed for filename: {filename}");
        }
    }
}
