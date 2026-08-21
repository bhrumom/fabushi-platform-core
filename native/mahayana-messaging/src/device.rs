use crate::actor::ActorId;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use thiserror::Error;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum DevicePlatform {
    Macos,
    Windows,
    Linux,
    Ios,
    Android,
    Web,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeviceSession {
    pub id: String,
    pub actor_id: ActorId,
    pub device_id: String,
    pub platform: DevicePlatform,
    pub device_name: String,
    pub app_version: String,
    pub created_at_ms: i64,
    pub last_active_at_ms: i64,
    pub revoked_at_ms: Option<i64>,
    pub push_token: Option<String>,
    pub sync_cursor: u64,
    pub trusted: bool,
}

impl DeviceSession {
    pub fn active(&self) -> bool {
        self.revoked_at_ms.is_none()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeviceSyncCursor {
    pub session_id: String,
    pub cursor: u64,
    pub acknowledged_at_ms: i64,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeviceRegistry {
    pub sessions: BTreeMap<String, DeviceSession>,
}

impl DeviceRegistry {
    pub fn register(&mut self, session: DeviceSession) -> Result<(), DeviceError> {
        if session.id.trim().is_empty() || session.device_id.trim().is_empty() {
            return Err(DeviceError::InvalidSession);
        }
        if self.sessions.contains_key(&session.id) {
            return Err(DeviceError::DuplicateSession(session.id));
        }
        self.sessions.insert(session.id.clone(), session);
        Ok(())
    }

    pub fn acknowledge(
        &mut self,
        session_id: &str,
        cursor: u64,
        now_ms: i64,
    ) -> Result<DeviceSyncCursor, DeviceError> {
        let session = self
            .sessions
            .get_mut(session_id)
            .ok_or_else(|| DeviceError::SessionNotFound(session_id.to_string()))?;
        if !session.active() {
            return Err(DeviceError::Revoked(session_id.to_string()));
        }
        session.sync_cursor = session.sync_cursor.max(cursor);
        session.last_active_at_ms = now_ms;
        Ok(DeviceSyncCursor {
            session_id: session_id.to_string(),
            cursor: session.sync_cursor,
            acknowledged_at_ms: now_ms,
        })
    }

    pub fn revoke(&mut self, session_id: &str, now_ms: i64) -> Result<(), DeviceError> {
        let session = self
            .sessions
            .get_mut(session_id)
            .ok_or_else(|| DeviceError::SessionNotFound(session_id.to_string()))?;
        session.revoked_at_ms = Some(now_ms);
        session.push_token = None;
        Ok(())
    }

    pub fn active_for_actor(&self, actor_id: &ActorId) -> Vec<&DeviceSession> {
        self.sessions
            .values()
            .filter(|session| &session.actor_id == actor_id && session.active())
            .collect()
    }

    pub fn minimum_active_cursor(&self, actor_id: &ActorId) -> Option<u64> {
        self.active_for_actor(actor_id)
            .into_iter()
            .map(|session| session.sync_cursor)
            .min()
    }
}

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum DeviceError {
    #[error("device session is invalid")]
    InvalidSession,
    #[error("device session {0} already exists")]
    DuplicateSession(String),
    #[error("device session {0} was not found")]
    SessionNotFound(String),
    #[error("device session {0} has been revoked")]
    Revoked(String),
}
