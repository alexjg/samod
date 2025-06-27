#[cfg(feature = "tokio")]
mod tokio_tests {
    use std::collections::HashMap;
    use tempfile::TempDir;

    use samod::storage::{Storage, TokioFilesystemStorage};
    use samod_core::StorageKey;

    fn init_logging() {
        let _ = tracing_subscriber::fmt()
            .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
            .try_init();
    }

    #[tokio::test]
    async fn test_tokio_storage_basic_operations() {
        init_logging();

        let temp_dir = TempDir::new().unwrap();
        let storage = TokioFilesystemStorage::new(temp_dir.path());

        let key = StorageKey::from(vec!["test"]);
        let data = b"hello world".to_vec();

        // Test put and load
        storage.put(key.clone(), data.clone()).await;
        let loaded = storage.load(key.clone()).await;
        assert_eq!(loaded, Some(data));

        // Test delete
        storage.delete(key.clone()).await;
        let loaded_after_delete = storage.load(key).await;
        assert_eq!(loaded_after_delete, None);
    }

    #[tokio::test]
    async fn test_tokio_storage_load_range() {
        init_logging();

        let temp_dir = TempDir::new().unwrap();
        let storage = TokioFilesystemStorage::new(temp_dir.path());

        let base_key = StorageKey::from(vec!["test_prefix"]);

        // Put multiple files with the same prefix
        let key1 = StorageKey::from(vec!["test_prefix", "file1"]);
        let key2 = StorageKey::from(vec!["test_prefix", "file2"]);
        let key3 = StorageKey::from(vec!["test_prefix", "subdir", "file3"]);

        let data1 = b"data1".to_vec();
        let data2 = b"data2".to_vec();
        let data3 = b"data3".to_vec();

        storage.put(key1.clone(), data1.clone()).await;
        storage.put(key2.clone(), data2.clone()).await;
        storage.put(key3.clone(), data3.clone()).await;

        // Load range
        let loaded_range = storage.load_range(base_key).await;

        let mut expected = HashMap::new();
        expected.insert(key1, data1);
        expected.insert(key2, data2);
        expected.insert(key3, data3);

        assert_eq!(loaded_range, expected);
    }

    #[tokio::test]
    async fn test_tokio_storage_nonexistent_file() {
        init_logging();

        let temp_dir = TempDir::new().unwrap();
        let storage = TokioFilesystemStorage::new(temp_dir.path());

        let key = StorageKey::from(vec!["nonexistent"]);
        let loaded = storage.load(key).await;
        assert_eq!(loaded, None);
    }

    #[tokio::test]
    async fn test_tokio_storage_empty_range() {
        init_logging();

        let temp_dir = TempDir::new().unwrap();
        let storage = TokioFilesystemStorage::new(temp_dir.path());

        let prefix = StorageKey::from(vec!["empty_prefix"]);
        let loaded_range = storage.load_range(prefix).await;
        assert!(loaded_range.is_empty());
    }

    #[tokio::test]
    async fn test_tokio_storage_nested_directories() {
        init_logging();

        let temp_dir = TempDir::new().unwrap();
        let storage = TokioFilesystemStorage::new(temp_dir.path());

        // Create a deeply nested key
        let key = StorageKey::from(vec!["level1", "level2", "level3", "file.txt"]);

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
        let key = StorageKey::from(vec!["abcdef", "file.txt"]);

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
    async fn test_tokio_storage_large_data() {
        init_logging();

        let temp_dir = TempDir::new().unwrap();
        let storage = TokioFilesystemStorage::new(temp_dir.path());

        let key = StorageKey::from(vec!["large_file"]);
        let data = vec![42u8; 1024 * 1024]; // 1MB of data

        storage.put(key.clone(), data.clone()).await;
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
            let key = StorageKey::from(vec![format!("concurrent_{}", i)]);
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
            let key = StorageKey::from(vec![filename]);
            let data = format!("data for {filename}").into_bytes();

            storage.put(key.clone(), data.clone()).await;
            let loaded = storage.load(key).await;
            assert_eq!(loaded, Some(data), "Failed for filename: {filename}");
        }
    }
}
