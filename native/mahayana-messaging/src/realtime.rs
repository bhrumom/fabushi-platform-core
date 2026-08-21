use crate::actor::ActorId;
use crate::conversation::ConversationId;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use thiserror::Error;

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct CallId(pub String);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum CallKind {
    Voice,
    Video,
    GroupVoice,
    GroupVideo,
    LiveStream,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum CallState {
    Ringing,
    Connecting,
    Active,
    Held,
    Ended,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CallParticipant {
    pub actor_id: ActorId,
    pub joined_at_ms: Option<i64>,
    pub left_at_ms: Option<i64>,
    pub muted: bool,
    pub video_enabled: bool,
    pub screen_sharing: bool,
    pub speaking: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IceServer {
    pub urls: Vec<String>,
    pub username: Option<String>,
    pub credential: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CallRoute {
    pub region: String,
    pub signaling_url: String,
    pub media_relay_urls: Vec<String>,
    pub ice_servers: Vec<IceServer>,
    pub end_to_end_encrypted: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CallSession {
    pub id: CallId,
    pub conversation_id: ConversationId,
    pub kind: CallKind,
    pub state: CallState,
    pub initiator_id: ActorId,
    pub participants: BTreeMap<ActorId, CallParticipant>,
    pub route: Option<CallRoute>,
    pub created_at_ms: i64,
    pub connected_at_ms: Option<i64>,
    pub ended_at_ms: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    tag = "type"
)]
pub enum CallSignal {
    Invite {
        call_id: CallId,
        conversation_id: ConversationId,
        kind: CallKind,
        from: ActorId,
        participants: Vec<ActorId>,
    },
    Ringing {
        call_id: CallId,
        actor_id: ActorId,
    },
    Accept {
        call_id: CallId,
        actor_id: ActorId,
    },
    Decline {
        call_id: CallId,
        actor_id: ActorId,
        reason: String,
    },
    SdpOffer {
        call_id: CallId,
        from: ActorId,
        to: ActorId,
        sdp: String,
    },
    SdpAnswer {
        call_id: CallId,
        from: ActorId,
        to: ActorId,
        sdp: String,
    },
    IceCandidate {
        call_id: CallId,
        from: ActorId,
        to: Option<ActorId>,
        candidate: String,
        sdp_mid: Option<String>,
        sdp_mline_index: Option<u16>,
    },
    MediaState {
        call_id: CallId,
        actor_id: ActorId,
        muted: bool,
        video_enabled: bool,
        screen_sharing: bool,
    },
    Speaking {
        call_id: CallId,
        actor_id: ActorId,
        active: bool,
    },
    Hangup {
        call_id: CallId,
        actor_id: ActorId,
        reason: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TypingState {
    pub conversation_id: ConversationId,
    pub actor_id: ActorId,
    pub action: String,
    pub expires_at_ms: i64,
}

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum RealtimeError {
    #[error("call {0:?} already exists")]
    DuplicateCall(CallId),
    #[error("call {0:?} does not exist")]
    CallNotFound(CallId),
    #[error("actor {actor_id:?} is not a participant in call {call_id:?}")]
    ParticipantNotFound { call_id: CallId, actor_id: ActorId },
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RealtimeState {
    pub calls: BTreeMap<CallId, CallSession>,
    pub typing: BTreeMap<(ConversationId, ActorId), TypingState>,
}

impl RealtimeState {
    pub fn create_call(&mut self, call: CallSession) -> Result<(), RealtimeError> {
        if self.calls.contains_key(&call.id) {
            return Err(RealtimeError::DuplicateCall(call.id));
        }
        self.calls.insert(call.id.clone(), call);
        Ok(())
    }

    pub fn set_participant_media(
        &mut self,
        call_id: &CallId,
        actor_id: &ActorId,
        muted: bool,
        video_enabled: bool,
        screen_sharing: bool,
    ) -> Result<(), RealtimeError> {
        let call = self
            .calls
            .get_mut(call_id)
            .ok_or_else(|| RealtimeError::CallNotFound(call_id.clone()))?;
        let participant = call.participants.get_mut(actor_id).ok_or_else(|| {
            RealtimeError::ParticipantNotFound {
                call_id: call_id.clone(),
                actor_id: actor_id.clone(),
            }
        })?;
        participant.muted = muted;
        participant.video_enabled = video_enabled;
        participant.screen_sharing = screen_sharing;
        Ok(())
    }

    pub fn set_typing(&mut self, state: TypingState) {
        self.typing.insert(
            (state.conversation_id.clone(), state.actor_id.clone()),
            state,
        );
    }

    pub fn expire_typing(&mut self, now_ms: i64) {
        self.typing.retain(|_, state| state.expires_at_ms > now_ms);
    }
}
