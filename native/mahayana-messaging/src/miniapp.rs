use crate::actor::ActorId;
use crate::conversation::ConversationId;
use crate::miniapp_service_call::{
    MiniAppServiceCallId, MiniAppServiceCallInput, MiniAppServiceCallMode, MiniAppServiceCallState,
};
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
    ServiceCall,
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
    SetHeaderColor {
        value: String,
    },
    SetBackgroundColor {
        value: String,
    },
    RequestTheme,
    RequestViewport,
    RequestIdentity,
    RequestLocation,
    RequestContact,
    RequestWriteAccess,
    ReadClipboard,
    WriteClipboard {
        text: String,
    },
    OpenExternal {
        url: String,
    },
    OpenInvoice {
        invoice_id: String,
    },
    SendData {
        data: String,
    },
    SendMessage {
        text: String,
    },
    Haptic {
        kind: String,
    },
    StartServiceCall {
        call_id: MiniAppServiceCallId,
        mode: MiniAppServiceCallMode,
    },
    SubmitServiceCallInput {
        call_id: MiniAppServiceCallId,
        input: MiniAppServiceCallInput,
    },
    EndServiceCall {
        call_id: MiniAppServiceCallId,
    },
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
    ServiceCallStatus {
        call_id: MiniAppServiceCallId,
        state: MiniAppServiceCallState,
    },
}

impl MiniAppRequest {
    pub fn required_permissions(&self) -> Vec<MiniAppPermission> {
        match self {
            Self::RequestIdentity => vec![MiniAppPermission::Identity],
            Self::RequestLocation => vec![MiniAppPermission::Location],
            Self::RequestContact => vec![MiniAppPermission::Contacts],
            Self::RequestWriteAccess => vec![MiniAppPermission::SendMessage],
            Self::ReadClipboard => vec![MiniAppPermission::ClipboardRead],
            Self::WriteClipboard { .. } => vec![MiniAppPermission::ClipboardWrite],
            Self::OpenExternal { .. } => vec![MiniAppPermission::OpenExternal],
            Self::OpenInvoice { .. } => vec![MiniAppPermission::Payments],
            Self::SendMessage { .. } | Self::SendData { .. } => {
                vec![MiniAppPermission::SendMessage]
            }
            Self::Haptic { .. } => vec![MiniAppPermission::Haptics],
            Self::StartServiceCall { mode, .. } => {
                let mut permissions = vec![MiniAppPermission::ServiceCall];
                if matches!(
                    mode,
                    MiniAppServiceCallMode::Voice | MiniAppServiceCallMode::Hybrid
                ) {
                    permissions.push(MiniAppPermission::Microphone);
                }
                permissions
            }
            Self::SubmitServiceCallInput { input, .. } => {
                let mut permissions = vec![MiniAppPermission::ServiceCall];
                if matches!(input, MiniAppServiceCallInput::SpeechTranscript { .. }) {
                    permissions.push(MiniAppPermission::Microphone);
                }
                permissions
            }
            Self::EndServiceCall { .. } => vec![MiniAppPermission::ServiceCall],
            Self::Ready
            | Self::Expand
            | Self::Close
            | Self::SetHeaderColor { .. }
            | Self::SetBackgroundColor { .. }
            | Self::RequestTheme
            | Self::RequestViewport => Vec::new(),
        }
    }

    pub fn required_permission(&self) -> Option<MiniAppPermission> {
        self.required_permissions().into_iter().next()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn voice_service_call_requires_service_call_and_microphone_permissions() {
        let request = MiniAppRequest::StartServiceCall {
            call_id: MiniAppServiceCallId::new("svc-call:1"),
            mode: MiniAppServiceCallMode::Voice,
        };
        assert_eq!(
            request.required_permissions(),
            vec![
                MiniAppPermission::ServiceCall,
                MiniAppPermission::Microphone
            ]
        );
    }

    #[test]
    fn text_service_call_does_not_require_microphone() {
        let request = MiniAppRequest::SubmitServiceCallInput {
            call_id: MiniAppServiceCallId::new("svc-call:1"),
            input: MiniAppServiceCallInput::ChatText {
                text: "查询余额".into(),
            },
        };
        assert_eq!(
            request.required_permissions(),
            vec![MiniAppPermission::ServiceCall]
        );
    }
}
