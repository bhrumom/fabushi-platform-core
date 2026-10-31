use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ActorId(pub String);

impl ActorId {
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    pub fn is_valid(&self) -> bool {
        let value = self.0.trim();
        !value.is_empty() && value.len() <= 160
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ActorKind {
    Human,
    Assistant,
    Bot,
    Service,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum PresenceStatus {
    Offline,
    Online,
    Away,
    DoNotDisturb,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Presence {
    pub status: PresenceStatus,
    pub last_seen_at_ms: Option<i64>,
    pub status_text: Option<String>,
}

impl Default for Presence {
    fn default() -> Self {
        Self {
            status: PresenceStatus::Offline,
            last_seen_at_ms: None,
            status_text: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Actor {
    pub id: ActorId,
    pub kind: ActorKind,
    pub display_name: String,
    pub username: Option<String>,
    pub avatar_url: Option<String>,
    pub bio: Option<String>,
    pub capabilities: Vec<String>,
    pub presence: Presence,
    pub verified: bool,
}

impl Actor {
    pub fn new(id: impl Into<String>, kind: ActorKind, name: impl Into<String>) -> Self {
        Self {
            id: ActorId::new(id),
            kind,
            display_name: name.into(),
            username: None,
            avatar_url: None,
            bio: None,
            capabilities: Vec::new(),
            presence: Presence::default(),
            verified: false,
        }
    }

    pub fn human(id: impl Into<String>, name: impl Into<String>) -> Self {
        Self::new(id, ActorKind::Human, name)
    }

    pub fn bot(id: impl Into<String>, name: impl Into<String>) -> Self {
        Self::new(id, ActorKind::Bot, name)
    }

    pub fn assistant(id: impl Into<String>, name: impl Into<String>) -> Self {
        Self::new(id, ActorKind::Assistant, name)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ParticipantRole {
    Owner,
    Admin,
    Member,
    Restricted,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Participant {
    pub actor_id: ActorId,
    pub role: ParticipantRole,
    pub joined_at_ms: i64,
    pub muted_until_ms: Option<i64>,
}
