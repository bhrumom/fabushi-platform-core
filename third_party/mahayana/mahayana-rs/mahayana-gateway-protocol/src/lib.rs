//! Mahayana's product-owned event gateway contract.
//!
//! This crate adapts the architectural lessons from the MIT-licensed
//! `NousResearch/hermes-agent` TUI gateway without embedding its Python runtime:
//! one ordered event vocabulary is shared by CLI/TUI/Desktop/mobile transports,
//! while Rust remains authoritative for sessions, tools, approvals and replay.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use thiserror::Error;

pub const GATEWAY_PROTOCOL_VERSION: u16 = 1;
pub const JSON_RPC_VERSION: &str = "2.0";
pub const EVENT_NOTIFICATION_METHOD: &str = "event";

/// Backend-owned events that product surfaces are allowed to project.
pub const GATEWAY_EVENT_NAMES: &[&str] = &[
    "message.start",
    "message.delta",
    "message.interim",
    "message.complete",
    "reasoning.delta",
    "thinking.delta",
    "tool.generating",
    "tool.start",
    "tool.complete",
    "approval.request",
    "clarify.request",
    "subagent.start",
    "subagent.progress",
    "subagent.complete",
];

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GatewayEventEnvelope {
    pub protocol_version: u16,
    pub session_id: String,
    pub turn_id: String,
    pub seq: u64,
    pub timestamp: String,
    pub replay_epoch: String,
    #[serde(flatten)]
    pub event: GatewayEvent,
}

impl GatewayEventEnvelope {
    pub fn new(
        session_id: impl Into<String>,
        turn_id: impl Into<String>,
        seq: u64,
        timestamp: impl Into<String>,
        replay_epoch: impl Into<String>,
        event: GatewayEvent,
    ) -> Self {
        Self {
            protocol_version: GATEWAY_PROTOCOL_VERSION,
            session_id: session_id.into(),
            turn_id: turn_id.into(),
            seq,
            timestamp: timestamp.into(),
            replay_epoch: replay_epoch.into(),
            event,
        }
    }

    pub fn kind(&self) -> &'static str {
        self.event.kind()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", content = "payload")]
pub enum GatewayEvent {
    #[serde(rename = "message.start")]
    MessageStart(MessageStartPayload),
    #[serde(rename = "message.delta")]
    MessageDelta(MessageDeltaPayload),
    #[serde(rename = "message.interim")]
    MessageInterim(MessageInterimPayload),
    #[serde(rename = "message.complete")]
    MessageComplete(MessageCompletePayload),
    #[serde(rename = "reasoning.delta")]
    ReasoningDelta(StreamDeltaPayload),
    #[serde(rename = "thinking.delta")]
    ThinkingDelta(StreamDeltaPayload),
    #[serde(rename = "tool.generating")]
    ToolGenerating(ToolGeneratingPayload),
    #[serde(rename = "tool.start")]
    ToolStart(ToolStartPayload),
    #[serde(rename = "tool.complete")]
    ToolComplete(ToolCompletePayload),
    #[serde(rename = "approval.request")]
    ApprovalRequest(ApprovalRequestPayload),
    #[serde(rename = "clarify.request")]
    ClarifyRequest(ClarifyRequestPayload),
    #[serde(rename = "subagent.start")]
    SubagentStart(SubagentPayload),
    #[serde(rename = "subagent.progress")]
    SubagentProgress(SubagentPayload),
    #[serde(rename = "subagent.complete")]
    SubagentComplete(SubagentPayload),
}

impl GatewayEvent {
    pub const fn kind(&self) -> &'static str {
        match self {
            Self::MessageStart(_) => "message.start",
            Self::MessageDelta(_) => "message.delta",
            Self::MessageInterim(_) => "message.interim",
            Self::MessageComplete(_) => "message.complete",
            Self::ReasoningDelta(_) => "reasoning.delta",
            Self::ThinkingDelta(_) => "thinking.delta",
            Self::ToolGenerating(_) => "tool.generating",
            Self::ToolStart(_) => "tool.start",
            Self::ToolComplete(_) => "tool.complete",
            Self::ApprovalRequest(_) => "approval.request",
            Self::ClarifyRequest(_) => "clarify.request",
            Self::SubagentStart(_) => "subagent.start",
            Self::SubagentProgress(_) => "subagent.progress",
            Self::SubagentComplete(_) => "subagent.complete",
        }
    }

    pub const fn is_streaming(&self) -> bool {
        matches!(
            self,
            Self::MessageDelta(_) | Self::ReasoningDelta(_) | Self::ThinkingDelta(_)
        )
    }

    /// Hermes flushes buffered token events before non-streaming semantic events.
    /// Exposing the same distinction lets every Mahayana transport preserve
    /// reasoning/text/tool order without moving sequencing logic into a renderer.
    pub const fn requires_stream_flush(&self) -> bool {
        !self.is_streaming()
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MessageStartPayload {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub role: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MessageDeltaPayload {
    pub text: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rendered: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StreamDeltaPayload {
    pub text: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MessageInterimPayload {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MessageCompletionStatus {
    #[default]
    Complete,
    Error,
    Interrupted,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MessageCompletePayload {
    #[serde(default)]
    pub status: MessageCompletionStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub partial: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub recoverable: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub usage: Option<Value>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolGeneratingPayload {
    pub tool_id: String,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolStartPayload {
    pub tool_id: String,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub args: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolCompletePayload {
    pub tool_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ApprovalRequestPayload {
    pub request_id: String,
    pub title: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metadata: Option<Value>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ClarifyRequestPayload {
    pub request_id: String,
    pub question: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub options: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SubagentPayload {
    pub subagent_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct JsonRpcEventNotification {
    pub jsonrpc: String,
    pub method: String,
    pub params: GatewayEventEnvelope,
}

impl JsonRpcEventNotification {
    pub fn new(params: GatewayEventEnvelope) -> Self {
        Self {
            jsonrpc: JSON_RPC_VERSION.to_owned(),
            method: EVENT_NOTIFICATION_METHOD.to_owned(),
            params,
        }
    }
}

/// Minimal replay cursor used by stdio/WebSocket transports before dispatching
/// to a surface. It rejects gaps and stale replays instead of silently rendering
/// a corrupt transcript.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GatewayReplayCursor {
    replay_epoch: Option<String>,
    next_seq: u64,
}

impl Default for GatewayReplayCursor {
    fn default() -> Self {
        Self {
            replay_epoch: None,
            next_seq: 0,
        }
    }
}

impl GatewayReplayCursor {
    pub fn observe(&mut self, event: &GatewayEventEnvelope) -> Result<(), GatewaySequenceError> {
        if event.protocol_version != GATEWAY_PROTOCOL_VERSION {
            return Err(GatewaySequenceError::UnsupportedProtocol {
                expected: GATEWAY_PROTOCOL_VERSION,
                actual: event.protocol_version,
            });
        }

        match self.replay_epoch.as_deref() {
            Some(epoch) if epoch != event.replay_epoch => {
                self.replay_epoch = Some(event.replay_epoch.clone());
                self.next_seq = event.seq.saturating_add(1);
                Ok(())
            }
            None => {
                self.replay_epoch = Some(event.replay_epoch.clone());
                self.next_seq = event.seq.saturating_add(1);
                Ok(())
            }
            Some(_) if event.seq == self.next_seq => {
                self.next_seq = self.next_seq.saturating_add(1);
                Ok(())
            }
            Some(_) if event.seq < self.next_seq => Err(GatewaySequenceError::Stale {
                expected: self.next_seq,
                actual: event.seq,
            }),
            Some(_) => Err(GatewaySequenceError::Gap {
                expected: self.next_seq,
                actual: event.seq,
            }),
        }
    }
}

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum GatewaySequenceError {
    #[error("unsupported gateway protocol version: expected {expected}, got {actual}")]
    UnsupportedProtocol { expected: u16, actual: u16 },
    #[error("stale gateway event: expected sequence {expected}, got {actual}")]
    Stale { expected: u64, actual: u64 },
    #[error("gateway event gap: expected sequence {expected}, got {actual}")]
    Gap { expected: u64, actual: u64 },
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn envelope(seq: u64, event: GatewayEvent) -> GatewayEventEnvelope {
        GatewayEventEnvelope::new(
            "session-1",
            "turn-1",
            seq,
            "2026-09-14T07:30:00Z",
            "epoch-a",
            event,
        )
    }

    #[test]
    fn event_catalog_matches_enum_names() {
        let variants = [
            GatewayEvent::MessageStart(MessageStartPayload::default()),
            GatewayEvent::MessageDelta(MessageDeltaPayload {
                text: "a".into(),
                rendered: None,
            }),
            GatewayEvent::MessageInterim(MessageInterimPayload::default()),
            GatewayEvent::MessageComplete(MessageCompletePayload::default()),
            GatewayEvent::ReasoningDelta(StreamDeltaPayload { text: "r".into() }),
            GatewayEvent::ThinkingDelta(StreamDeltaPayload { text: "t".into() }),
            GatewayEvent::ToolGenerating(ToolGeneratingPayload {
                tool_id: "1".into(),
                name: "read".into(),
                description: None,
            }),
            GatewayEvent::ToolStart(ToolStartPayload {
                tool_id: "1".into(),
                name: "read".into(),
                args: None,
                description: None,
            }),
            GatewayEvent::ToolComplete(ToolCompletePayload {
                tool_id: "1".into(),
                name: None,
                result: None,
                error: None,
            }),
            GatewayEvent::ApprovalRequest(ApprovalRequestPayload {
                request_id: "a".into(),
                title: "approve".into(),
                description: None,
                tool_id: None,
                metadata: None,
            }),
            GatewayEvent::ClarifyRequest(ClarifyRequestPayload {
                request_id: "c".into(),
                question: "?".into(),
                options: vec![],
            }),
            GatewayEvent::SubagentStart(SubagentPayload {
                subagent_id: "s".into(),
                title: None,
                detail: None,
                status: None,
            }),
            GatewayEvent::SubagentProgress(SubagentPayload {
                subagent_id: "s".into(),
                title: None,
                detail: None,
                status: None,
            }),
            GatewayEvent::SubagentComplete(SubagentPayload {
                subagent_id: "s".into(),
                title: None,
                detail: None,
                status: None,
            }),
        ];
        let names: Vec<_> = variants.iter().map(GatewayEvent::kind).collect();
        assert_eq!(names, GATEWAY_EVENT_NAMES);
    }

    #[test]
    fn json_rpc_notification_has_stable_wire_shape() {
        let notification = JsonRpcEventNotification::new(envelope(
            7,
            GatewayEvent::MessageDelta(MessageDeltaPayload {
                text: "你好".into(),
                rendered: None,
            }),
        ));
        assert_eq!(
            serde_json::to_value(notification).unwrap(),
            json!({
                "jsonrpc": "2.0",
                "method": "event",
                "params": {
                    "protocolVersion": 1,
                    "sessionId": "session-1",
                    "turnId": "turn-1",
                    "seq": 7,
                    "timestamp": "2026-09-14T07:30:00Z",
                    "replayEpoch": "epoch-a",
                    "type": "message.delta",
                    "payload": { "text": "你好" }
                }
            })
        );
    }

    #[test]
    fn replay_cursor_rejects_gaps_and_stale_events_but_accepts_new_epoch() {
        let mut cursor = GatewayReplayCursor::default();
        cursor
            .observe(&envelope(
                10,
                GatewayEvent::MessageStart(MessageStartPayload::default()),
            ))
            .unwrap();
        cursor
            .observe(&envelope(
                11,
                GatewayEvent::MessageDelta(MessageDeltaPayload {
                    text: "a".into(),
                    rendered: None,
                }),
            ))
            .unwrap();
        assert_eq!(
            cursor.observe(&envelope(
                13,
                GatewayEvent::MessageDelta(MessageDeltaPayload {
                    text: "b".into(),
                    rendered: None,
                })
            )),
            Err(GatewaySequenceError::Gap {
                expected: 12,
                actual: 13
            })
        );
        assert_eq!(
            cursor.observe(&envelope(
                10,
                GatewayEvent::MessageDelta(MessageDeltaPayload {
                    text: "c".into(),
                    rendered: None,
                })
            )),
            Err(GatewaySequenceError::Stale {
                expected: 12,
                actual: 10
            })
        );

        let mut restarted = envelope(
            2,
            GatewayEvent::MessageStart(MessageStartPayload::default()),
        );
        restarted.replay_epoch = "epoch-b".into();
        cursor.observe(&restarted).unwrap();
    }

    #[test]
    fn non_stream_events_require_token_flush() {
        assert!(
            !GatewayEvent::MessageDelta(MessageDeltaPayload {
                text: "x".into(),
                rendered: None
            })
            .requires_stream_flush()
        );
        assert!(
            GatewayEvent::ToolStart(ToolStartPayload {
                tool_id: "t".into(),
                name: "read".into(),
                args: None,
                description: None
            })
            .requires_stream_flush()
        );
        assert!(
            GatewayEvent::MessageComplete(MessageCompletePayload::default())
                .requires_stream_flush()
        );
    }
}
