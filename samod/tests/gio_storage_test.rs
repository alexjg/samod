#[cfg(feature = "gio")]
mod gio_tests {
    use std::collections::HashMap;
    use tempfile::TempDir;

    use samod::storage::{GioFilesystemStorage, Storage};
    use samod_core::StorageKey;

    fn init_logging() {
        let _ = tracing_subscriber::fmt()
            .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
            .try_init();
    }

    #[test]
    fn test_gio_storage_basic_operations() {
        init_logging();

        let main_context = glib::MainContext::new();
        main_context.block_on(async {
            let temp_dir = TempDir::new().unwrap();
            let storage = GioFilesystemStorage::new(temp_dir.path());

            let key = StorageKey::from_parts(vec!["test"]).unwrap();
            let data = b"hello world".to_vec();

            // Test put and load
            storage.put(key.clone(), data.clone()).await;
            let loaded = storage.load(key.clone()).await;
            assert_eq!(loaded, Some(data));

            // Test delete
            storage.delete(key.clone()).await;
            let loaded_after_delete = storage.load(key).await;
            assert_eq!(loaded_after_delete, None);
        });
    }

    #[test]
    fn test_gio_storage_load_range() {
        init_logging();

        let main_context = glib::MainContext::new();
        main_context.block_on(async {
            let temp_dir = TempDir::new().unwrap();
            let storage = GioFilesystemStorage::new(temp_dir.path());

            let base_key = StorageKey::from_parts(vec!["test_prefix"]).unwrap();

            // Put multiple files with the same prefix
            let key1 = StorageKey::from_parts(vec!["test_prefix", "file1"]).unwrap();
            let key2 = StorageKey::from_parts(vec!["test_prefix", "file2"]).unwrap();
            let key3 = StorageKey::from_parts(vec!["test_prefix", "subdir", "file3"]).unwrap();

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
        });
    }

    #[test]
    fn test_gio_storage_nonexistent_file() {
        init_logging();

        let main_context = glib::MainContext::new();
        main_context.block_on(async {
            let temp_dir = TempDir::new().unwrap();
            let storage = GioFilesystemStorage::new(temp_dir.path());

            let key = StorageKey::from_parts(vec!["nonexistent"]).unwrap();
            let loaded = storage.load(key).await;
            assert_eq!(loaded, None);
        });
    }

    #[test]
    fn test_gio_storage_empty_range() {
        init_logging();

        let main_context = glib::MainContext::new();
        main_context.block_on(async {
            let temp_dir = TempDir::new().unwrap();
            let storage = GioFilesystemStorage::new(temp_dir.path());

            let prefix = StorageKey::from_parts(vec!["empty_prefix"]).unwrap();
            let loaded_range = storage.load_range(prefix).await;
            assert!(loaded_range.is_empty());
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
