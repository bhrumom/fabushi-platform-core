use crate::message::MediaRef;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use thiserror::Error;

/// Platform-neutral cache bookkeeping for media already materialized on a client.
/// Actual file I/O remains owned by the platform adapter; this type enforces the
/// shared byte budget and deterministic eviction semantics.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MediaCacheEntry {
    pub media: MediaRef,
    pub local_path: String,
    pub size_bytes: u64,
    pub last_accessed_ms: i64,
    pub pinned: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MediaCache {
    max_bytes: u64,
    entries: BTreeMap<String, MediaCacheEntry>,
}

impl MediaCache {
    pub fn new(max_bytes: u64) -> Result<Self, MediaCacheError> {
        if max_bytes == 0 {
            return Err(MediaCacheError::InvalidBudget);
        }
        Ok(Self {
            max_bytes,
            entries: BTreeMap::new(),
        })
    }

    pub fn max_bytes(&self) -> u64 {
        self.max_bytes
    }

    pub fn total_bytes(&self) -> u64 {
        self.entries
            .values()
            .fold(0u64, |total, entry| total.saturating_add(entry.size_bytes))
    }

    pub fn entries(&self) -> &BTreeMap<String, MediaCacheEntry> {
        &self.entries
    }

    pub fn get(&mut self, media_id: &str, now_ms: i64) -> Option<&MediaCacheEntry> {
        let entry = self.entries.get_mut(media_id)?;
        entry.last_accessed_ms = now_ms;
        Some(entry)
    }

    pub fn insert(
        &mut self,
        media: MediaRef,
        local_path: impl Into<String>,
        size_bytes: u64,
        now_ms: i64,
        pinned: bool,
    ) -> Result<Vec<MediaCacheEntry>, MediaCacheError> {
        if media.id.trim().is_empty() {
            return Err(MediaCacheError::InvalidMediaId);
        }
        let local_path = local_path.into();
        if local_path.trim().is_empty() {
            return Err(MediaCacheError::InvalidLocalPath);
        }
        if size_bytes > self.max_bytes {
            return Err(MediaCacheError::EntryTooLarge {
                size_bytes,
                max_bytes: self.max_bytes,
            });
        }

        let key = media.id.clone();
        let previous = self.entries.insert(
            key.clone(),
            MediaCacheEntry {
                media,
                local_path,
                size_bytes,
                last_accessed_ms: now_ms,
                pinned,
            },
        );

        match self.evict_to_budget(Some(&key)) {
            Ok(evicted) => Ok(evicted),
            Err(error) => {
                match previous {
                    Some(previous) => {
                        self.entries.insert(key, previous);
                    }
                    None => {
                        self.entries.remove(&key);
                    }
                }
                Err(error)
            }
        }
    }

    pub fn remove(&mut self, media_id: &str) -> Option<MediaCacheEntry> {
        self.entries.remove(media_id)
    }

    pub fn set_pinned(&mut self, media_id: &str, pinned: bool) -> Result<(), MediaCacheError> {
        let entry = self
            .entries
            .get_mut(media_id)
            .ok_or_else(|| MediaCacheError::NotFound(media_id.to_string()))?;
        entry.pinned = pinned;
        Ok(())
    }

    fn evict_to_budget(
        &mut self,
        protected_media_id: Option<&str>,
    ) -> Result<Vec<MediaCacheEntry>, MediaCacheError> {
        let mut evicted = Vec::new();
        while self.total_bytes() > self.max_bytes {
            let candidate = self
                .entries
                .values()
                .filter(|entry| !entry.pinned)
                .filter(|entry| protected_media_id != Some(entry.media.id.as_str()))
                .min_by_key(|entry| (entry.last_accessed_ms, entry.media.id.clone()))
                .map(|entry| entry.media.id.clone());
            let Some(candidate) = candidate else {
                return Err(MediaCacheError::PinnedEntriesExceedBudget);
            };
            if let Some(entry) = self.entries.remove(&candidate) {
                evicted.push(entry);
            }
        }
        Ok(evicted)
    }
}

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum MediaCacheError {
    #[error("media cache byte budget must be greater than zero")]
    InvalidBudget,
    #[error("media cache entry requires a media id")]
    InvalidMediaId,
    #[error("media cache entry requires a local path")]
    InvalidLocalPath,
    #[error("media cache entry is {size_bytes} bytes but budget is {max_bytes} bytes")]
    EntryTooLarge { size_bytes: u64, max_bytes: u64 },
    #[error("media cache pinned/protected entries exceed the configured byte budget")]
    PinnedEntriesExceedBudget,
    #[error("media cache entry {0} was not found")]
    NotFound(String),
}
