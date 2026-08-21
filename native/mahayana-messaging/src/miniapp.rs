use crate::actor::ActorId;
use crate::conversation::ConversationId;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum MiniAppPermission {
    Identity,
    Theme,
    ClipboardRead,
    ClipboardWrite,
    Camera,
    Microphone,
    Location,
    Contacts,
    Files,
    Notifications,
    Haptics,
    Payments,
    OpenExternal,
    SendMessage,
    ReadConversation,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MiniAppManifest {
    pub id: String,
    pub name: String,
    pub version: String,
    pub description: String,
    pub icon_url: Option<String>,
    pub start_url: String,
    pub allowed_origins: Vec<String>,
    pub requested_permissions: Vec<MiniAppPermission>,
    pub bot_actor_id: Option<ActorId>,
    pub verified: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MiniAppGrant {
    pub mini_app_id: String,
    pub actor_id: ActorId,
    pub permissions: Vec<MiniAppPermission>,
    pub granted_at_ms: i64,
    pub expires_at_ms: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MiniAppSession {
    pub id: String,
    pub mini_app_id: String,
    pub actor_id: ActorId,
    pub conversation_id: Option<ConversationId>,
    pub start_parameter: Option<String>,
    pub granted_permissions: Vec<MiniAppPermission>,
    pub opened_at_ms: i64,
    pub expires_at_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    tag = "type"
)]
pub enum MiniAppRequest {
    Ready,
    Expand,
    Close,
    SetHeaderColor { value: String },
    SetBackgroundColor { value: String },
    RequestTheme,
    RequestViewport,
    RequestIdentity,
    RequestLocation,
    RequestContact,
    RequestWriteAccess,
    ReadClipboard,
    WriteClipboard { text: String },
    OpenExternal { url: String },
    OpenInvoice { invoice_id: String },
    SendData { data: String },
    SendMessage { text: String },
    Haptic { kind: String },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    tag = "type"
)]
pub enum MiniAppResponse {
    Ok,
    Error {
        code: String,
        message: String,
    },
    Theme {
        color_scheme: String,
        values: Vec<(String, String)>,
    },
    Viewport {
        width: f64,
        height: f64,
        stable_height: f64,
        expanded: bool,
    },
    Identity {
        actor_id: ActorId,
        display_name: String,
        username: Option<String>,
    },
    Location {
        latitude: f64,
        longitude: f64,
    },
    Contact {
        actor_id: Option<ActorId>,
        display_name: String,
    },
    Clipboard {
        text: Option<String>,
    },
    InvoiceStatus {
        invoice_id: String,
        status: String,
    },
}

impl MiniAppRequest {
    pub fn required_permission(&self) -> Option<MiniAppPermission> {
        match self {
            Self::RequestIdentity => Some(MiniAppPermission::Identity),
            Self::RequestLocation => Some(MiniAppPermission::Location),
            Self::RequestContact => Some(MiniAppPermission::Contacts),
            Self::RequestWriteAccess => Some(MiniAppPermission::SendMessage),
            Self::ReadClipboard => Some(MiniAppPermission::ClipboardRead),
            Self::WriteClipboard { .. } => Some(MiniAppPermission::ClipboardWrite),
            Self::OpenExternal { .. } => Some(MiniAppPermission::OpenExternal),
            Self::OpenInvoice { .. } => Some(MiniAppPermission::Payments),
            Self::SendMessage { .. } | Self::SendData { .. } => {
                Some(MiniAppPermission::SendMessage)
            }
            Self::Haptic { .. } => Some(MiniAppPermission::Haptics),
            Self::Ready
            | Self::Expand
            | Self::Close
            | Self::SetHeaderColor { .. }
            | Self::SetBackgroundColor { .. }
            | Self::RequestTheme
            | Self::RequestViewport => None,
        }
    }
}
