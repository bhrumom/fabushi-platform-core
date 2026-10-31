use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use thiserror::Error;

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct TransferId(pub String);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum TransferDirection {
    Upload,
    Download,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum TransferState {
    Queued,
    Resolving,
    Transferring,
    Paused,
    Verifying,
    Completed,
    Failed,
    Cancelled,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MediaDescriptor {
    pub id: String,
    pub file_name: String,
    pub mime_type: String,
    pub size_bytes: u64,
    pub content_hash: Option<String>,
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub duration_ms: Option<u64>,
    pub thumbnail_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MediaTransfer {
    pub id: TransferId,
    pub direction: TransferDirection,
    pub media: MediaDescriptor,
    pub state: TransferState,
    pub local_path: Option<String>,
    pub remote_locator: Option<String>,
    pub chunk_size: u32,
    pub transferred_bytes: u64,
    pub verified_bytes: u64,
    pub retry_count: u32,
    pub created_at_ms: i64,
    pub updated_at_ms: i64,
    pub error: Option<String>,
}

impl MediaTransfer {
    pub fn progress_percent(&self) -> u8 {
        if self.media.size_bytes == 0 {
            return if self.state == TransferState::Completed {
                100
            } else {
                0
            };
        }
        let percent = self
            .transferred_bytes
            .saturating_mul(100)
            .checked_div(self.media.size_bytes)
            .unwrap_or_default();
        percent.min(100) as u8
    }

    pub fn remaining_bytes(&self) -> u64 {
        self.media.size_bytes.saturating_sub(self.transferred_bytes)
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MediaTransferQueue {
    pub transfers: BTreeMap<TransferId, MediaTransfer>,
}

impl MediaTransferQueue {
    pub fn enqueue(&mut self, transfer: MediaTransfer) -> Result<(), MediaError> {
        if transfer.chunk_size == 0 {
            return Err(MediaError::InvalidChunkSize);
        }
        if self.transfers.contains_key(&transfer.id) {
            return Err(MediaError::DuplicateTransfer(transfer.id));
        }
        self.transfers.insert(transfer.id.clone(), transfer);
        Ok(())
    }

    pub fn start(&mut self, id: &TransferId, now_ms: i64) -> Result<(), MediaError> {
        let transfer = self.require_mut(id)?;
        match transfer.state {
            TransferState::Queued | TransferState::Paused | TransferState::Failed => {
                transfer.state = TransferState::Transferring;
                transfer.updated_at_ms = now_ms;
                transfer.error = None;
                Ok(())
            }
            state => Err(MediaError::InvalidTransition {
                from: state,
                to: TransferState::Transferring,
            }),
        }
    }

    pub fn acknowledge_chunk(
        &mut self,
        id: &TransferId,
        offset: u64,
        length: u32,
        now_ms: i64,
    ) -> Result<(), MediaError> {
        let transfer = self.require_mut(id)?;
        if transfer.state != TransferState::Transferring {
            return Err(MediaError::InvalidTransition {
                from: transfer.state,
                to: TransferState::Transferring,
            });
        }
        if offset != transfer.transferred_bytes {
            return Err(MediaError::UnexpectedOffset {
                expected: transfer.transferred_bytes,
                actual: offset,
            });
        }
        transfer.transferred_bytes = transfer
            .transferred_bytes
            .saturating_add(u64::from(length))
            .min(transfer.media.size_bytes);
        transfer.updated_at_ms = now_ms;
        if transfer.transferred_bytes == transfer.media.size_bytes {
            transfer.state = TransferState::Verifying;
        }
        Ok(())
    }

    pub fn complete_verification(
        &mut self,
        id: &TransferId,
        verified_bytes: u64,
        remote_locator: Option<String>,
        now_ms: i64,
    ) -> Result<(), MediaError> {
        let transfer = self.require_mut(id)?;
        if transfer.state != TransferState::Verifying {
            return Err(MediaError::InvalidTransition {
                from: transfer.state,
                to: TransferState::Completed,
            });
        }
        if verified_bytes != transfer.media.size_bytes {
            return Err(MediaError::VerificationMismatch {
                expected: transfer.media.size_bytes,
                actual: verified_bytes,
            });
        }
        transfer.verified_bytes = verified_bytes;
        transfer.remote_locator = remote_locator.or_else(|| transfer.remote_locator.clone());
        transfer.state = TransferState::Completed;
        transfer.updated_at_ms = now_ms;
        Ok(())
    }

    pub fn pause(&mut self, id: &TransferId, now_ms: i64) -> Result<(), MediaError> {
        let transfer = self.require_mut(id)?;
        if transfer.state != TransferState::Transferring {
            return Err(MediaError::InvalidTransition {
                from: transfer.state,
                to: TransferState::Paused,
            });
        }
        transfer.state = TransferState::Paused;
        transfer.updated_at_ms = now_ms;
        Ok(())
    }

    pub fn fail(
        &mut self,
        id: &TransferId,
        error: impl Into<String>,
        now_ms: i64,
    ) -> Result<(), MediaError> {
        let transfer = self.require_mut(id)?;
        if matches!(
            transfer.state,
            TransferState::Completed | TransferState::Cancelled
        ) {
            return Err(MediaError::TerminalTransfer(id.clone()));
        }
        transfer.state = TransferState::Failed;
        transfer.retry_count = transfer.retry_count.saturating_add(1);
        transfer.error = Some(error.into());
        transfer.updated_at_ms = now_ms;
        Ok(())
    }

    pub fn cancel(&mut self, id: &TransferId, now_ms: i64) -> Result<(), MediaError> {
        let transfer = self.require_mut(id)?;
        if transfer.state == TransferState::Completed {
            return Err(MediaError::TerminalTransfer(id.clone()));
        }
        transfer.state = TransferState::Cancelled;
        transfer.updated_at_ms = now_ms;
        Ok(())
    }

    fn require_mut(&mut self, id: &TransferId) -> Result<&mut MediaTransfer, MediaError> {
        self.transfers
            .get_mut(id)
            .ok_or_else(|| MediaError::TransferNotFound(id.clone()))
    }
}

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum MediaError {
    #[error("media transfer {0:?} already exists")]
    DuplicateTransfer(TransferId),
    #[error("media transfer {0:?} was not found")]
    TransferNotFound(TransferId),
    #[error("media transfer chunk size must be greater than zero")]
    InvalidChunkSize,
    #[error("media transfer cannot transition from {from:?} to {to:?}")]
    InvalidTransition {
        from: TransferState,
        to: TransferState,
    },
    #[error("media transfer expected offset {expected}, got {actual}")]
    UnexpectedOffset { expected: u64, actual: u64 },
    #[error("media verification expected {expected} bytes, got {actual}")]
    VerificationMismatch { expected: u64, actual: u64 },
    #[error("media transfer {0:?} is already terminal")]
    TerminalTransfer(TransferId),
}
