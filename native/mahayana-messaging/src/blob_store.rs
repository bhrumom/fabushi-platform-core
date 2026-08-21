use serde::{Deserialize, Serialize};
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use thiserror::Error;

pub const MAX_BLOB_RANGE_BYTES: u64 = 8 * 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct BlobId(pub String);

impl BlobId {
    pub fn new(value: impl Into<String>) -> Result<Self, BlobStoreError> {
        let value = value.into();
        if value.is_empty()
            || value.len() > 128
            || !value
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
            || value == "."
            || value == ".."
        {
            return Err(BlobStoreError::InvalidBlobId(value));
        }
        Ok(Self(value))
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BlobMetadata {
    pub id: BlobId,
    pub file_name: String,
    pub mime_type: String,
    pub size_bytes: u64,
    pub content_hash: Option<String>,
    pub created_at_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BlobUploadStatus {
    pub id: BlobId,
    pub size_bytes: u64,
    pub uploaded_bytes: u64,
    pub complete: bool,
}

#[derive(Debug, Clone)]
pub struct FileBlobStore {
    root: PathBuf,
}

impl FileBlobStore {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn begin_upload(
        &self,
        metadata: &BlobMetadata,
    ) -> Result<BlobUploadStatus, BlobStoreError> {
        validate_metadata(metadata)?;
        fs::create_dir_all(&self.root)?;
        let final_path = self.blob_path(&metadata.id);
        if final_path.exists() {
            let existing = self.metadata(&metadata.id)?;
            if existing == *metadata {
                return Ok(BlobUploadStatus {
                    id: metadata.id.clone(),
                    size_bytes: metadata.size_bytes,
                    uploaded_bytes: metadata.size_bytes,
                    complete: true,
                });
            }
            return Err(BlobStoreError::AlreadyExists(metadata.id.clone()));
        }
        let part_path = self.part_path(&metadata.id);
        if !part_path.exists() {
            File::create(&part_path)?.sync_all()?;
        }
        self.write_metadata_atomic(&self.part_metadata_path(&metadata.id), metadata)?;
        let uploaded_bytes = part_path.metadata()?.len();
        if uploaded_bytes > metadata.size_bytes {
            return Err(BlobStoreError::UploadTooLarge {
                id: metadata.id.clone(),
                expected: metadata.size_bytes,
                actual: uploaded_bytes,
            });
        }
        Ok(BlobUploadStatus {
            id: metadata.id.clone(),
            size_bytes: metadata.size_bytes,
            uploaded_bytes,
            complete: false,
        })
    }

    pub fn upload_status(&self, id: &BlobId) -> Result<BlobUploadStatus, BlobStoreError> {
        let final_path = self.blob_path(id);
        if final_path.exists() {
            let metadata = self.metadata(id)?;
            return Ok(BlobUploadStatus {
                id: id.clone(),
                size_bytes: metadata.size_bytes,
                uploaded_bytes: final_path.metadata()?.len(),
                complete: true,
            });
        }
        let metadata = self.read_metadata(&self.part_metadata_path(id))?;
        let uploaded_bytes = self.part_path(id).metadata()?.len();
        Ok(BlobUploadStatus {
            id: id.clone(),
            size_bytes: metadata.size_bytes,
            uploaded_bytes,
            complete: false,
        })
    }

    pub fn append_chunk(
        &self,
        id: &BlobId,
        offset: u64,
        bytes: &[u8],
    ) -> Result<BlobUploadStatus, BlobStoreError> {
        if bytes.is_empty() {
            return Err(BlobStoreError::EmptyChunk);
        }
        let metadata = self.read_metadata(&self.part_metadata_path(id))?;
        let part_path = self.part_path(id);
        let current = part_path.metadata()?.len();
        if current != offset {
            return Err(BlobStoreError::UnexpectedOffset {
                id: id.clone(),
                expected: current,
                actual: offset,
            });
        }
        let next = current
            .checked_add(bytes.len() as u64)
            .ok_or(BlobStoreError::SizeOverflow)?;
        if next > metadata.size_bytes {
            return Err(BlobStoreError::UploadTooLarge {
                id: id.clone(),
                expected: metadata.size_bytes,
                actual: next,
            });
        }
        let mut file = OpenOptions::new().append(true).open(&part_path)?;
        file.write_all(bytes)?;
        file.sync_data()?;
        Ok(BlobUploadStatus {
            id: id.clone(),
            size_bytes: metadata.size_bytes,
            uploaded_bytes: next,
            complete: false,
        })
    }

    pub fn finish_upload(&self, id: &BlobId) -> Result<BlobMetadata, BlobStoreError> {
        let metadata = self.read_metadata(&self.part_metadata_path(id))?;
        let part_path = self.part_path(id);
        let actual = part_path.metadata()?.len();
        if actual != metadata.size_bytes {
            return Err(BlobStoreError::IncompleteUpload {
                id: id.clone(),
                expected: metadata.size_bytes,
                actual,
            });
        }
        let final_path = self.blob_path(id);
        fs::rename(&part_path, &final_path)?;
        let final_metadata_path = self.metadata_path(id);
        fs::rename(self.part_metadata_path(id), &final_metadata_path)?;
        sync_directory(&self.root);
        Ok(metadata)
    }

    pub fn metadata(&self, id: &BlobId) -> Result<BlobMetadata, BlobStoreError> {
        self.read_metadata(&self.metadata_path(id))
    }

    pub fn read_range(
        &self,
        id: &BlobId,
        offset: u64,
        length: u64,
    ) -> Result<Vec<u8>, BlobStoreError> {
        if length == 0 || length > MAX_BLOB_RANGE_BYTES {
            return Err(BlobStoreError::InvalidRangeLength(length));
        }
        let path = self.blob_path(id);
        let size = path.metadata()?.len();
        if offset >= size {
            return Err(BlobStoreError::RangeOutOfBounds {
                id: id.clone(),
                offset,
                size,
            });
        }
        let readable = length.min(size.saturating_sub(offset));
        let mut file = File::open(path)?;
        file.seek(SeekFrom::Start(offset))?;
        let mut bytes = vec![0u8; readable as usize];
        file.read_exact(&mut bytes)?;
        Ok(bytes)
    }

    pub fn delete(&self, id: &BlobId) -> Result<(), BlobStoreError> {
        let mut found = false;
        for path in [
            self.blob_path(id),
            self.metadata_path(id),
            self.part_path(id),
            self.part_metadata_path(id),
        ] {
            match fs::remove_file(path) {
                Ok(()) => found = true,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => return Err(error.into()),
            }
        }
        if !found {
            return Err(BlobStoreError::NotFound(id.clone()));
        }
        sync_directory(&self.root);
        Ok(())
    }

    fn blob_path(&self, id: &BlobId) -> PathBuf {
        self.root.join(format!("{}.blob", id.0))
    }

    fn metadata_path(&self, id: &BlobId) -> PathBuf {
        self.root.join(format!("{}.json", id.0))
    }

    fn part_path(&self, id: &BlobId) -> PathBuf {
        self.root.join(format!("{}.part", id.0))
    }

    fn part_metadata_path(&self, id: &BlobId) -> PathBuf {
        self.root.join(format!("{}.part.json", id.0))
    }

    fn read_metadata(&self, path: &Path) -> Result<BlobMetadata, BlobStoreError> {
        let bytes = fs::read(path).map_err(|error| {
            if error.kind() == std::io::ErrorKind::NotFound {
                BlobStoreError::MissingMetadata(path.to_path_buf())
            } else {
                BlobStoreError::Io(error)
            }
        })?;
        Ok(serde_json::from_slice(&bytes)?)
    }

    fn write_metadata_atomic(
        &self,
        path: &Path,
        metadata: &BlobMetadata,
    ) -> Result<(), BlobStoreError> {
        let temporary = path.with_extension("json.tmp");
        let payload = serde_json::to_vec(metadata)?;
        {
            let mut file = File::create(&temporary)?;
            file.write_all(&payload)?;
            file.sync_all()?;
        }
        fs::rename(temporary, path)?;
        sync_directory(&self.root);
        Ok(())
    }
}

fn validate_metadata(metadata: &BlobMetadata) -> Result<(), BlobStoreError> {
    BlobId::new(metadata.id.0.clone())?;
    if metadata.file_name.trim().is_empty() {
        return Err(BlobStoreError::InvalidFileName);
    }
    if metadata.mime_type.trim().is_empty() {
        return Err(BlobStoreError::InvalidMimeType);
    }
    Ok(())
}

fn sync_directory(path: &Path) {
    if let Ok(directory) = File::open(path) {
        let _ = directory.sync_all();
    }
}

#[derive(Debug, Error)]
pub enum BlobStoreError {
    #[error("blob id is invalid: {0}")]
    InvalidBlobId(String),
    #[error("blob file name is invalid")]
    InvalidFileName,
    #[error("blob MIME type is invalid")]
    InvalidMimeType,
    #[error("blob {0:?} already exists with different metadata")]
    AlreadyExists(BlobId),
    #[error("blob {0:?} was not found")]
    NotFound(BlobId),
    #[error("blob metadata is missing at {0}")]
    MissingMetadata(PathBuf),
    #[error("blob chunk must not be empty")]
    EmptyChunk,
    #[error("blob size overflow")]
    SizeOverflow,
    #[error("blob {id:?} expected offset {expected}, received {actual}")]
    UnexpectedOffset {
        id: BlobId,
        expected: u64,
        actual: u64,
    },
    #[error("blob {id:?} expected at most {expected} bytes, received {actual}")]
    UploadTooLarge {
        id: BlobId,
        expected: u64,
        actual: u64,
    },
    #[error("blob {id:?} is incomplete: expected {expected} bytes, found {actual}")]
    IncompleteUpload {
        id: BlobId,
        expected: u64,
        actual: u64,
    },
    #[error("blob range length {0} is invalid")]
    InvalidRangeLength(u64),
    #[error("blob {id:?} range starts at {offset}, file size is {size}")]
    RangeOutOfBounds { id: BlobId, offset: u64, size: u64 },
    #[error("blob store I/O failed: {0}")]
    Io(#[from] std::io::Error),
    #[error("blob store JSON failed: {0}")]
    Json(#[from] serde_json::Error),
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temporary_root() -> PathBuf {
        let suffix = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!("fabushi-blob-store-{suffix}"))
    }

    #[test]
    fn supports_resumable_upload_and_range_download() {
        let root = temporary_root();
        let store = FileBlobStore::new(&root);
        let id = BlobId::new("media-1").unwrap();
        let metadata = BlobMetadata {
            id: id.clone(),
            file_name: "hello.txt".into(),
            mime_type: "text/plain".into(),
            size_bytes: 11,
            content_hash: None,
            created_at_ms: 1,
        };
        let status = store.begin_upload(&metadata).unwrap();
        assert_eq!(status.uploaded_bytes, 0);
        store.append_chunk(&id, 0, b"hello ").unwrap();
        let status = store.upload_status(&id).unwrap();
        assert_eq!(status.uploaded_bytes, 6);
        store.append_chunk(&id, 6, b"world").unwrap();
        store.finish_upload(&id).unwrap();
        assert_eq!(store.read_range(&id, 6, 5).unwrap(), b"world");
        store.delete(&id).unwrap();
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn rejects_path_traversal_blob_ids() {
        assert!(BlobId::new("../secret").is_err());
        assert!(BlobId::new("folder/file").is_err());
    }
}
