pub mod plugin_bundle_archive {
    use flate2::read::GzDecoder;
    use std::fs;
    use std::fs::OpenOptions;
    use std::io;
    use std::path::{Component, Path, PathBuf};
    use tar::Archive;
    use thiserror::Error;

    #[derive(Debug, Error)]
    pub enum PluginBundleArchiveError {
        #[error("plugin archive destination already exists: {0}")]
        DestinationExists(PathBuf),
        #[error("plugin archive contains an unsafe path: {0}")]
        UnsafePath(PathBuf),
        #[error("plugin archive contains an unsupported entry type: {0}")]
        UnsupportedEntry(PathBuf),
        #[error("plugin archive exceeds {limit} unpacked bytes ({actual} bytes)")]
        TooLarge { limit: u64, actual: u64 },
        #[error(
            "plugin archive entry size mismatch for {path}: expected {expected}, copied {actual}"
        )]
        SizeMismatch {
            path: PathBuf,
            expected: u64,
            actual: u64,
        },
        #[error(transparent)]
        Io(#[from] io::Error),
    }

    pub fn unpack_plugin_bundle_tar_gz(
        archive: &[u8],
        destination: &Path,
        max_unpacked_bytes: u64,
    ) -> Result<(), PluginBundleArchiveError> {
        if destination.exists() {
            return Err(PluginBundleArchiveError::DestinationExists(
                destination.to_path_buf(),
            ));
        }
        fs::create_dir_all(destination)?;

        let decoder = GzDecoder::new(archive);
        let mut tar = Archive::new(decoder);
        let mut total_bytes = 0u64;

        for entry in tar.entries()? {
            let mut entry = entry?;
            let archive_path = entry.path()?.into_owned();
            let relative = safe_relative_path(&archive_path)?;
            if relative.as_os_str().is_empty() {
                continue;
            }
            let output = destination.join(&relative);
            let entry_type = entry.header().entry_type();

            if entry_type.is_dir() {
                fs::create_dir_all(&output)?;
                continue;
            }
            if !entry_type.is_file() {
                return Err(PluginBundleArchiveError::UnsupportedEntry(archive_path));
            }

            let declared_size = entry.header().size()?;
            total_bytes = total_bytes.checked_add(declared_size).ok_or(
                PluginBundleArchiveError::TooLarge {
                    limit: max_unpacked_bytes,
                    actual: u64::MAX,
                },
            )?;
            if total_bytes > max_unpacked_bytes {
                return Err(PluginBundleArchiveError::TooLarge {
                    limit: max_unpacked_bytes,
                    actual: total_bytes,
                });
            }

            if let Some(parent) = output.parent() {
                fs::create_dir_all(parent)?;
            }
            let mut file = OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&output)?;
            let copied = io::copy(&mut entry, &mut file)?;
            if copied != declared_size {
                return Err(PluginBundleArchiveError::SizeMismatch {
                    path: relative,
                    expected: declared_size,
                    actual: copied,
                });
            }
        }

        Ok(())
    }

    fn safe_relative_path(path: &Path) -> Result<PathBuf, PluginBundleArchiveError> {
        if path.is_absolute() {
            return Err(PluginBundleArchiveError::UnsafePath(path.to_path_buf()));
        }
        let mut normalized = PathBuf::new();
        for component in path.components() {
            match component {
                Component::Normal(value) => normalized.push(value),
                Component::CurDir => {}
                Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
                    return Err(PluginBundleArchiveError::UnsafePath(path.to_path_buf()));
                }
            }
        }
        Ok(normalized)
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        use flate2::Compression;
        use flate2::write::GzEncoder;
        use std::time::{SystemTime, UNIX_EPOCH};
        use tar::{Builder, EntryType, Header};

        fn destination(name: &str) -> PathBuf {
            let nonce = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("clock")
                .as_nanos();
            std::env::temp_dir().join(format!("mahayana-archive-{name}-{nonce}"))
        }

        fn regular_archive(path: &str, bytes: &[u8]) -> Vec<u8> {
            let encoder = GzEncoder::new(Vec::new(), Compression::default());
            let mut builder = Builder::new(encoder);
            let mut header = Header::new_gnu();
            header.set_entry_type(EntryType::Regular);
            header.set_mode(0o644);
            header.set_size(bytes.len() as u64);
            header.set_cksum();
            builder
                .append_data(&mut header, path, bytes)
                .expect("append fixture");
            builder.finish().expect("finish tar");
            builder
                .into_inner()
                .expect("encoder")
                .finish()
                .expect("gzip")
        }

        #[test]
        fn extracts_regular_file_inside_fresh_destination() {
            let root = destination("roundtrip");
            let archive = regular_archive("nested/plugin.json", br#"{"name":"demo"}"#);
            unpack_plugin_bundle_tar_gz(&archive, &root, 1024).expect("extract");
            assert_eq!(
                fs::read(root.join("nested/plugin.json")).expect("read"),
                br#"{"name":"demo"}"#
            );
            fs::remove_dir_all(root).expect("cleanup");
        }

        #[test]
        fn enforces_declared_unpacked_size_before_writing() {
            let root = destination("limit");
            let archive = regular_archive("plugin.json", b"12345");
            let error = unpack_plugin_bundle_tar_gz(&archive, &root, 4).expect_err("limit");
            assert!(matches!(error, PluginBundleArchiveError::TooLarge { .. }));
            fs::remove_dir_all(root).expect("cleanup");
        }

        #[test]
        fn rejects_non_regular_entries() {
            let root = destination("symlink");
            let encoder = GzEncoder::new(Vec::new(), Compression::default());
            let mut builder = Builder::new(encoder);
            let mut header = Header::new_gnu();
            header.set_entry_type(EntryType::Symlink);
            header.set_mode(0o777);
            header.set_size(0);
            header.set_link_name("../outside").expect("link name");
            header.set_cksum();
            builder
                .append_data(&mut header, "link", io::empty())
                .expect("append symlink");
            builder.finish().expect("finish tar");
            let archive = builder
                .into_inner()
                .expect("encoder")
                .finish()
                .expect("gzip");
            let error = unpack_plugin_bundle_tar_gz(&archive, &root, 1024).expect_err("reject");
            assert!(matches!(
                error,
                PluginBundleArchiveError::UnsupportedEntry(_)
            ));
            fs::remove_dir_all(root).expect("cleanup");
        }
    }
}
