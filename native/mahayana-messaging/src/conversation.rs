use crate::actor::{ActorId, Participant};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ConversationId(pub String);

impl ConversationId {
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    pub fn is_valid(&self) -> bool {
        let value = self.0.trim();
        !value.is_empty() && value.len() <= 200
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ConversationKind {
    Direct,
    Group,
    Channel,
    SavedMessages,
    Secret,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum HistoryVisibility {
    NewMembersOnly,
    AllMembers,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NotificationSettings {
    pub muted_until_ms: Option<i64>,
    pub sound: Option<String>,
    pub show_preview: bool,
    pub notify_mentions: bool,
}

impl Default for NotificationSettings {
    fn default() -> Self {
        Self {
            muted_until_ms: None,
            sound: None,
            show_preview: true,
            notify_mentions: true,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConversationPermissions {
    pub can_send_messages: bool,
    pub can_send_media: bool,
    pub can_send_polls: bool,
    pub can_add_members: bool,
    pub can_pin_messages: bool,
    pub can_manage_topics: bool,
    pub can_manage_calls: bool,
}

impl Default for ConversationPermissions {
    fn default() -> Self {
        Self {
            can_send_messages: true,
            can_send_media: true,
            can_send_polls: true,
            can_add_members: true,
            can_pin_messages: true,
            can_manage_topics: true,
            can_manage_calls: true,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Topic {
    pub id: String,
    pub title: String,
    pub icon: Option<String>,
    pub created_by: ActorId,
    pub closed: bool,
    pub hidden: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Conversation {
    pub id: ConversationId,
    pub kind: ConversationKind,
    pub title: String,
    pub description: Option<String>,
    pub avatar_url: Option<String>,
    pub participants: Vec<Participant>,
    pub owner_id: Option<ActorId>,
    pub last_message_id: Option<String>,
    pub last_read_message_id: Option<String>,
    pub unread_count: u32,
    pub mention_count: u32,
    pub pinned_message_ids: Vec<String>,
    pub notification_settings: NotificationSettings,
    pub permissions: ConversationPermissions,
    pub history_visibility: HistoryVisibility,
    pub topics: Vec<Topic>,
    pub folder_ids: Vec<String>,
    pub archived: bool,
    pub pinned: bool,
    pub marked_unread: bool,
    pub created_at_ms: i64,
    pub updated_at_ms: i64,
}

impl Conversation {
    pub fn direct(
        id: impl Into<String>,
        title: impl Into<String>,
        participants: Vec<Participant>,
        now_ms: i64,
    ) -> Self {
        Self {
            id: ConversationId::new(id),
            kind: ConversationKind::Direct,
            title: title.into(),
            description: None,
            avatar_url: None,
            participants,
            owner_id: None,
            last_message_id: None,
            last_read_message_id: None,
            unread_count: 0,
            mention_count: 0,
            pinned_message_ids: Vec::new(),
            notification_settings: NotificationSettings::default(),
            permissions: ConversationPermissions::default(),
            history_visibility: HistoryVisibility::AllMembers,
            topics: Vec::new(),
            folder_ids: Vec::new(),
            archived: false,
            pinned: false,
            marked_unread: false,
            created_at_ms: now_ms,
            updated_at_ms: now_ms,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConversationDraft {
    pub conversation_id: ConversationId,
    pub actor_id: ActorId,
    pub text: String,
    pub reply_to_message_id: Option<String>,
    pub updated_at_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConversationFolder {
    pub id: String,
    pub title: String,
    pub icon: Option<String>,
    pub conversation_ids: Vec<ConversationId>,
    pub include_contacts: bool,
    pub include_bots: bool,
    pub include_groups: bool,
    pub include_channels: bool,
    pub exclude_muted: bool,
    pub exclude_read: bool,
    pub exclude_archived: bool,
}
