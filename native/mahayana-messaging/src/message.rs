use crate::actor::ActorId;
use crate::conversation::ConversationId;
use crate::secret_chat::EncryptedSecretMessage;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct MessageId(pub String);

impl MessageId {
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ClientMessageId(pub String);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TextEntity {
    pub offset_utf16: u32,
    pub length_utf16: u32,
    pub kind: TextEntityKind,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    tag = "type",
    content = "value"
)]
pub enum TextEntityKind {
    Mention,
    MentionActor(ActorId),
    Hashtag,
    Url,
    Email,
    PhoneNumber,
    BotCommand,
    Bold,
    Italic,
    Underline,
    Strikethrough,
    Spoiler,
    Code,
    Pre { language: Option<String> },
    TextUrl(String),
    CustomEmoji(String),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FormattedText {
    pub text: String,
    pub entities: Vec<TextEntity>,
}

impl FormattedText {
    pub fn plain(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            entities: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MediaRef {
    pub id: String,
    pub file_name: Option<String>,
    pub mime_type: Option<String>,
    pub size_bytes: Option<u64>,
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub duration_ms: Option<u64>,
    pub thumbnail_id: Option<String>,
    pub local_path: Option<String>,
    pub remote_url: Option<String>,
    pub content_hash: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PollOption {
    pub id: String,
    pub text: String,
    pub voter_count: u32,
    pub chosen: bool,
    pub correct: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InlineButton {
    pub text: String,
    pub action: InlineButtonAction,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    tag = "type"
)]
pub enum InlineButtonAction {
    Callback {
        data: String,
    },
    Url {
        url: String,
    },
    MiniApp {
        mini_app_id: String,
        start_parameter: Option<String>,
    },
    Pay {
        invoice_id: String,
    },
    SwitchInline {
        query: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReplyMarkup {
    pub rows: Vec<Vec<InlineButton>>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    tag = "type",
    content = "data"
)]
pub enum MessageContent {
    Text {
        text: FormattedText,
    },
    Photo {
        media: MediaRef,
        caption: FormattedText,
        spoiler: bool,
    },
    Video {
        media: MediaRef,
        caption: FormattedText,
        spoiler: bool,
        streaming: bool,
    },
    Animation {
        media: MediaRef,
        caption: FormattedText,
    },
    Audio {
        media: MediaRef,
        caption: FormattedText,
        title: Option<String>,
        performer: Option<String>,
    },
    Voice {
        media: MediaRef,
        caption: FormattedText,
        waveform: Vec<u8>,
    },
    VideoNote {
        media: MediaRef,
    },
    Document {
        media: MediaRef,
        caption: FormattedText,
    },
    Sticker {
        media: MediaRef,
        emoji: Option<String>,
        set_id: Option<String>,
    },
    Contact {
        actor_id: Option<ActorId>,
        display_name: String,
        phone_number: Option<String>,
    },
    Location {
        latitude: f64,
        longitude: f64,
        live_until_ms: Option<i64>,
    },
    Venue {
        latitude: f64,
        longitude: f64,
        title: String,
        address: String,
    },
    Poll {
        question: FormattedText,
        options: Vec<PollOption>,
        anonymous: bool,
        multiple_answers: bool,
        quiz: bool,
    },
    Dice {
        emoji: String,
        value: u8,
    },
    Story {
        story_id: String,
    },
    Invoice {
        invoice_id: String,
    },
    MiniApp {
        mini_app_id: String,
        title: String,
        start_parameter: Option<String>,
    },
    Secret {
        envelope: EncryptedSecretMessage,
    },
    Service {
        action: String,
        text: Option<String>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    tag = "state"
)]
pub enum DeliveryState {
    Pending { client_message_id: ClientMessageId },
    Sent,
    Delivered,
    Read,
    Failed { code: String, retryable: bool },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReactionSummary {
    pub reaction: String,
    pub count: u32,
    pub chosen_by_me: bool,
    pub recent_actor_ids: Vec<ActorId>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Message {
    pub id: MessageId,
    pub conversation_id: ConversationId,
    pub sender_id: ActorId,
    pub content: MessageContent,
    pub reply_to_message_id: Option<MessageId>,
    pub thread_root_message_id: Option<MessageId>,
    pub forward_origin: Option<String>,
    pub reply_markup: Option<ReplyMarkup>,
    pub reactions: Vec<ReactionSummary>,
    pub delivery_state: DeliveryState,
    pub created_at_ms: i64,
    pub edited_at_ms: Option<i64>,
    pub scheduled_at_ms: Option<i64>,
    pub silent: bool,
    pub protected_content: bool,
    pub pinned: bool,
    pub deleted: bool,
}
