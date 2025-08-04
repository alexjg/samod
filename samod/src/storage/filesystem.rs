#[cfg(any(feature = "tokio", feature = "gio"))]
pub fn key_to_path(key: &samod_core::StorageKey) -> std::path::PathBuf {
    let mut result = std::path::PathBuf::new();
    for (index, component) in key.into_iter().enumerate() {
        // splay the first key out by the first two characters
        if index == 0 {
            let first_two = component.chars().take(2).collect::<String>();
            let remaining = component.chars().skip(2).collect::<String>();
            result.push(first_two);
            result.push(remaining);
        } else {
            result.push(component);
        }
    }
    result
}

#[cfg(feature = "tokio")]
pub mod tokio {
    use std::{
        collections::HashMap,
        path::{Path, PathBuf},
    };

    use crate::storage::Storage;

    use super::key_to_path;

    #[derive(Clone, Debug)]
    pub struct FilesystemStorage {
        data_dir: PathBuf,
    }

    impl FilesystemStorage {
        pub fn new<P: AsRef<Path>>(data_dir: P) -> Self {
            Self {
                data_dir: data_dir.as_ref().to_path_buf(),
            }
        }

        fn key_to_path(&self, key: &samod_core::StorageKey) -> PathBuf {
            let mut path = self.data_dir.clone();
            path.push(key_to_path(key));
            path
        }
    }

    impl Storage for FilesystemStorage {
        fn load(
            &self,
            key: samod_core::StorageKey,
        ) -> impl Future<Output = Option<Vec<u8>>> + Send {
            let path = self.key_to_path(&key);
            async move {
                let meta = tokio::fs::metadata(&path).await.ok()?;
                if !meta.is_file() {
                    return None;
                }
                let result = tokio::fs::read(path).await.unwrap();
                Some(result)
            }
        }

        fn load_range(
            &self,
            prefix: samod_core::StorageKey,
        ) -> impl Future<Output = std::collections::HashMap<samod_core::StorageKey, Vec<u8>>> + Send
        {
            let path = self.key_to_path(&prefix);
            async move {
                let Some(meta) = tokio::fs::metadata(&path).await.ok() else {
                    return HashMap::new(); // Return empty map if path does not exist
                };
                if !meta.is_dir() {
                    return HashMap::new();
                }
                let mut result = HashMap::new();
                // Now walk the directory, recursively
                let mut to_visit = vec![(path, prefix)];
                while let Some((next_path, key_prefix)) = to_visit.pop() {
                    let mut entries = tokio::fs::read_dir(&next_path).await.unwrap();
                    while let Some(entry) = entries.next_entry().await.unwrap() {
                        let Some(filename) = entry.file_name().to_str().map(|s| s.to_string())
                        else {
                            continue; // Skip entries with non-UTF8 names
                        };
                        let entry_path = entry.path();
                        let Some(entry_meta) = tokio::fs::metadata(&entry_path).await.ok() else {
                            continue;
                        };
                        let next_key_prefix = key_prefix.with_component(filename);
                        if entry_meta.is_dir() {
                            to_visit.push((entry_path, next_key_prefix));
                        } else if entry_path.is_file() {
                            let data = tokio::fs::read(&entry_path).await.unwrap();
                            result.insert(next_key_prefix, data);
                        }
                    }
                }
                result
            }
        }

        fn put(
            &self,
            key: samod_core::StorageKey,
            data: Vec<u8>,
        ) -> impl Future<Output = ()> + Send {
            let path = self.key_to_path(&key);
            async move {
                let parent = path.parent().unwrap();
                tokio::fs::create_dir_all(parent).await.unwrap();
                tokio::fs::write(path, data).await.unwrap();
            }
        }

        fn delete(&self, key: samod_core::StorageKey) -> impl Future<Output = ()> + Send {
            let path = self.key_to_path(&key);
            async move {
                tokio::fs::remove_file(path).await.unwrap();
            }
        }
    }
}

#[cfg(feature = "gio")]
pub mod gio {
    use std::{
        collections::HashMap,
        path::{Path, PathBuf},
    };

    use crate::storage::Storage;
    use gio::prelude::*;

    use super::key_to_path;

    #[derive(Clone, Debug)]
    pub struct FilesystemStorage {
        data_dir: PathBuf,
    }

    impl FilesystemStorage {
        pub fn new<P: AsRef<Path>>(data_dir: P) -> Self {
            Self {
                data_dir: data_dir.as_ref().to_path_buf(),
            }
        }

        fn key_to_path(&self, key: &samod_core::StorageKey) -> PathBuf {
            let mut path = self.data_dir.clone();
            path.push(key_to_path(key));
            path
        }
    }

    impl Storage for FilesystemStorage {
        fn load(
            &self,
            key: samod_core::StorageKey,
        ) -> impl std::future::Future<Output = Option<Vec<u8>>> + Send {
            let path = self.key_to_path(&key);
            async move {
                let (tx, rx) = futures::channel::oneshot::channel();
                std::thread::spawn(move || {
                    let file = gio::File::for_path(&path);

                    // Check if file exists and is a regular file
                    let result = (|| {
                        let file_info = file
                            .query_info("*", gio::FileQueryInfoFlags::NONE, gio::Cancellable::NONE)
                            .ok()?;

                        if file_info.file_type() != gio::FileType::Regular {
                            return None;
                        }

                        // Read the file contents
                        let (contents, _) = file.load_contents(gio::Cancellable::NONE).ok()?;
                        Some(contents.to_vec())
                    })();

                    let _ = tx.send(result);
                });
                rx.await.unwrap_or(None)
            }
        }

        fn load_range(
            &self,
            prefix: samod_core::StorageKey,
        ) -> impl std::future::Future<Output = HashMap<samod_core::StorageKey, Vec<u8>>> + Send
        {
            let path = self.key_to_path(&prefix);
            async move {
                let (tx, rx) = futures::channel::oneshot::channel();
                std::thread::spawn(move || {
                    let mut result = HashMap::new();
                    let dir = gio::File::for_path(&path);

                    // Check if directory exists
                    let dir_info = match dir.query_info(
                        "*",
                        gio::FileQueryInfoFlags::NONE,
                        gio::Cancellable::NONE,
                    ) {
                        Ok(info) => info,
                        Err(_) => {
                            let _ = tx.send(result);
                            return;
                        }
                    };

                    if dir_info.file_type() != gio::FileType::Directory {
                        let _ = tx.send(result);
                        return;
                    }

                    // Recursively walk the directory
                    let mut to_visit = vec![(dir, prefix)];
                    while let Some((current_dir, key_prefix)) = to_visit.pop() {
                        let enumerator = match current_dir.enumerate_children(
                            "*",
                            gio::FileQueryInfoFlags::NONE,
                            gio::Cancellable::NONE,
                        ) {
                            Ok(enumerator) => enumerator,
                            Err(_) => continue,
                        };

                        while let Some(file_info) =
                            enumerator.next_file(gio::Cancellable::NONE).unwrap_or(None)
                        {
                            let filename = file_info.name();
                            let Some(filename_str) = filename.to_str() else {
                                continue;
                            };

                            let child = current_dir.child(&filename);
                            let next_key_prefix =
                                key_prefix.with_component(filename_str.to_string());

                            match file_info.file_type() {
                                gio::FileType::Directory => {
                                    to_visit.push((child, next_key_prefix));
                                }
                                gio::FileType::Regular => {
                                    if let Ok((contents, _)) =
                                        child.load_contents(gio::Cancellable::NONE)
                                    {
                                        result.insert(next_key_prefix, contents.to_vec());
                                    }
                                }
                                _ => {}
                            }
                        }
                    }

                    let _ = tx.send(result);
                });
                rx.await.unwrap_or_else(|_| HashMap::new())
            }
        }

        fn put(
            &self,
            key: samod_core::StorageKey,
            data: Vec<u8>,
        ) -> impl std::future::Future<Output = ()> + Send {
            let path = self.key_to_path(&key);
            async move {
                let (tx, rx) = futures::channel::oneshot::channel();
                std::thread::spawn(move || {
                    let file = gio::File::for_path(&path);

                    // Create parent directories if they don't exist
                    if let Some(parent) = file.parent() {
                        let _ = parent.make_directory_with_parents(gio::Cancellable::NONE);
                    }

                    // Write the file
                    let bytes = glib::Bytes::from(&data);
                    let _ = file.replace_contents(
                        &bytes,
                        None,
                        false,
                        gio::FileCreateFlags::REPLACE_DESTINATION,
                        gio::Cancellable::NONE,
                    );

                    let _ = tx.send(());
                });
                let _ = rx.await;
            }
        }

        fn delete(
            &self,
            key: samod_core::StorageKey,
        ) -> impl std::future::Future<Output = ()> + Send {
            let path = self.key_to_path(&key);
            async move {
                let (tx, rx) = futures::channel::oneshot::channel();
                std::thread::spawn(move || {
                    let file = gio::File::for_path(&path);
                    let _ = file.delete(gio::Cancellable::NONE);
                    let _ = tx.send(());
                });
                let _ = rx.await;
            }
        }
    }
}
