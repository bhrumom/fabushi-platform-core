use crate::actor::ActorId;
use crate::conversation::ConversationId;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum NotificationImportance {
    Silent,
    Normal,
    High,
    Urgent,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct QuietHours {
    pub enabled: bool,
    pub start_minute_local: u16,
    pub end_minute_local: u16,
    pub allow_mentions: bool,
    pub allow_calls: bool,
}

impl QuietHours {
    pub fn active_at(&self, minute_local: u16) -> bool {
        if !self.enabled {
            return false;
        }
        let minute = minute_local % (24 * 60);
        let start = self.start_minute_local % (24 * 60);
        let end = self.end_minute_local % (24 * 60);
        if start == end {
            return true;
        }
        if start < end {
            minute >= start && minute < end
        } else {
            minute >= start || minute < end
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConversationNotificationRule {
    pub conversation_id: ConversationId,
    pub muted_until_ms: Option<i64>,
    pub sound: Option<String>,
    pub show_preview: bool,
    pub notify_mentions: bool,
    pub importance: NotificationImportance,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NotificationCandidate {
    pub id: String,
    pub conversation_id: ConversationId,
    pub sender_id: ActorId,
    pub title: String,
    pub body: String,
    pub created_at_ms: i64,
    pub mentioned_actor_ids: BTreeSet<ActorId>,
    pub is_call: bool,
    pub silent_message: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NotificationDecision {
    pub deliver: bool,
    pub show_preview: bool,
    pub sound: Option<String>,
    pub importance: NotificationImportance,
    pub reason: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NotificationPolicy {
    pub quiet_hours: Option<QuietHours>,
    pub conversation_rules: BTreeMap<ConversationId, ConversationNotificationRule>,
}

impl NotificationPolicy {
    pub fn decide(
        &self,
        candidate: &NotificationCandidate,
        current_actor_id: &ActorId,
        now_ms: i64,
        minute_local: u16,
    ) -> NotificationDecision {
        let rule = self.conversation_rules.get(&candidate.conversation_id);
        let importance = rule
            .map(|value| value.importance)
            .unwrap_or(NotificationImportance::Normal);
        let show_preview = rule.map(|value| value.show_preview).unwrap_or(true);
        let sound = rule.and_then(|value| value.sound.clone());
        if candidate.silent_message {
            return NotificationDecision {
                deliver: true,
                show_preview,
                sound: None,
                importance: NotificationImportance::Silent,
                reason: "silent-message".into(),
            };
        }
        if rule
            .and_then(|value| value.muted_until_ms)
            .is_some_and(|until| until > now_ms)
        {
            let mentioned = candidate.mentioned_actor_ids.contains(current_actor_id);
            if !(mentioned && rule.is_some_and(|value| value.notify_mentions)) {
                return NotificationDecision {
                    deliver: false,
                    show_preview: false,
                    sound: None,
                    importance: NotificationImportance::Silent,
                    reason: "conversation-muted".into(),
                };
            }
        }
        if let Some(quiet) = &self.quiet_hours {
            if quiet.active_at(minute_local) {
                let mention_allowed = quiet.allow_mentions
                    && candidate.mentioned_actor_ids.contains(current_actor_id);
                let call_allowed = quiet.allow_calls && candidate.is_call;
                if !mention_allowed && !call_allowed {
                    return NotificationDecision {
                        deliver: false,
                        show_preview: false,
                        sound: None,
                        importance: NotificationImportance::Silent,
                        reason: "quiet-hours".into(),
                    };
                }
            }
        }
        NotificationDecision {
            deliver: true,
            show_preview,
            sound,
            importance,
            reason: "deliver".into(),
        }
    }
}
