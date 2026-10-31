use crate::actor::ActorId;
use crate::conversation::ConversationId;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};
use thiserror::Error;

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct MiniAppServiceCallId(pub String);

impl MiniAppServiceCallId {
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
pub enum MiniAppServiceCallMode {
    Voice,
    Text,
    Hybrid,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum MiniAppServiceCallState {
    Connecting,
    Active,
    Held,
    Ended,
    Failed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum MiniAppServiceCallInputSource {
    Dtmf,
    Speech,
    Chat,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    tag = "type"
)]
pub enum MiniAppServiceCallInput {
    Dtmf {
        digits: String,
    },
    SpeechTranscript {
        text: String,
        is_final: bool,
        confidence_bps: Option<u16>,
    },
    ChatText {
        text: String,
    },
}

impl MiniAppServiceCallInput {
    pub fn source(&self) -> MiniAppServiceCallInputSource {
        match self {
            Self::Dtmf { .. } => MiniAppServiceCallInputSource::Dtmf,
            Self::SpeechTranscript { .. } => MiniAppServiceCallInputSource::Speech,
            Self::ChatText { .. } => MiniAppServiceCallInputSource::Chat,
        }
    }

    pub fn transcript_text(&self) -> Option<&str> {
        match self {
            Self::Dtmf { digits } => Some(digits),
            Self::SpeechTranscript { text, .. } | Self::ChatText { text } => Some(text),
        }
    }

    pub fn normalized_dtmf_digits(&self) -> Option<&str> {
        match self {
            Self::Dtmf { digits } => Some(digits.as_str()),
            Self::ChatText { text } => {
                let trimmed = text.trim();
                valid_dtmf(trimmed).then_some(trimmed)
            }
            Self::SpeechTranscript { .. } => None,
        }
    }

    pub fn requires_mcp_resolution(&self) -> bool {
        match self {
            Self::Dtmf { .. } => false,
            Self::SpeechTranscript { is_final, .. } => *is_final,
            Self::ChatText { text } => !valid_dtmf(text.trim()),
        }
    }

    fn validate(&self) -> Result<(), MiniAppServiceCallError> {
        match self {
            Self::Dtmf { digits } => {
                if !valid_dtmf(digits) {
                    return Err(MiniAppServiceCallError::InvalidDtmf);
                }
            }
            Self::SpeechTranscript {
                text,
                confidence_bps,
                ..
            } => {
                if text.trim().is_empty() || text.len() > 16_000 {
                    return Err(MiniAppServiceCallError::InvalidTranscript);
                }
                if confidence_bps.is_some_and(|value| value > 10_000) {
                    return Err(MiniAppServiceCallError::InvalidConfidence);
                }
            }
            Self::ChatText { text } => {
                if text.trim().is_empty() || text.len() > 16_000 {
                    return Err(MiniAppServiceCallError::InvalidTranscript);
                }
            }
        }
        Ok(())
    }
}

fn valid_dtmf(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 32
        && value
            .chars()
            .all(|ch| ch.is_ascii_digit() || matches!(ch, '*' | '#'))
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct MiniAppMcpToolAnnotations {
    pub read_only_hint: bool,
    pub destructive_hint: bool,
    pub open_world_hint: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MiniAppMcpToolCapability {
    pub name: String,
    pub title: Option<String>,
    pub description: Option<String>,
    pub input_schema: Option<Value>,
    #[serde(default)]
    pub annotations: MiniAppMcpToolAnnotations,
}

impl MiniAppMcpToolCapability {
    pub fn is_valid(&self) -> bool {
        let name = self.name.trim();
        !name.is_empty() && name.len() <= 200
    }
}

pub type MiniAppMcpArguments = BTreeMap<String, Value>;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MiniAppMcpDtmfRoute {
    pub digits: String,
    pub tool_name: String,
    #[serde(default)]
    pub arguments: MiniAppMcpArguments,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MiniAppServiceCallTurn {
    pub sequence: u64,
    pub actor_id: ActorId,
    pub input: MiniAppServiceCallInput,
    pub created_at_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MiniAppMcpResolveRequest {
    pub call_id: MiniAppServiceCallId,
    pub mini_app_id: String,
    pub conversation_id: ConversationId,
    pub turn_sequence: u64,
    pub input: MiniAppServiceCallInput,
    pub available_tools: Vec<MiniAppMcpToolCapability>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    tag = "type"
)]
pub enum MiniAppMcpToolResolution {
    Invoke {
        tool_name: String,
        #[serde(default)]
        arguments: MiniAppMcpArguments,
    },
    Unavailable {
        reason: String,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MiniAppMcpInvocationRequest {
    pub call_id: MiniAppServiceCallId,
    pub mini_app_id: String,
    pub conversation_id: ConversationId,
    pub turn_sequence: u64,
    pub tool_name: String,
    #[serde(default)]
    pub arguments: MiniAppMcpArguments,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MiniAppMcpInvocationResult {
    pub invocation_id: String,
    pub tool_name: String,
    pub success: bool,
    pub structured_content: Option<Value>,
    pub error: Option<String>,
    pub spoken_response: Option<String>,
    pub display_response: Option<String>,
    pub completed_at_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    tag = "type"
)]
pub enum MiniAppServiceCallEffect {
    ResolveMcpTool {
        request: MiniAppMcpResolveRequest,
    },
    InvokeMcpTool {
        request: MiniAppMcpInvocationRequest,
    },
    McpCapabilityUnavailable {
        call_id: MiniAppServiceCallId,
        mini_app_id: String,
        turn_sequence: u64,
        reason: String,
    },
    AppendConversationTranscript {
        conversation_id: ConversationId,
        actor_id: ActorId,
        source: MiniAppServiceCallInputSource,
        text: String,
        final_segment: bool,
        turn_sequence: u64,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MiniAppServiceCallSession {
    pub id: MiniAppServiceCallId,
    pub mini_app_id: String,
    pub mini_app_session_id: Option<String>,
    pub conversation_id: ConversationId,
    pub caller_actor_id: ActorId,
    pub service_actor_id: Option<ActorId>,
    pub mode: MiniAppServiceCallMode,
    pub state: MiniAppServiceCallState,
    pub mcp_tools: Vec<MiniAppMcpToolCapability>,
    pub dtmf_routes: Vec<MiniAppMcpDtmfRoute>,
    pub turns: Vec<MiniAppServiceCallTurn>,
    pub mcp_results: Vec<MiniAppMcpInvocationResult>,
    pub started_at_ms: i64,
    pub updated_at_ms: i64,
    pub ended_at_ms: Option<i64>,
}

impl MiniAppServiceCallSession {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        id: MiniAppServiceCallId,
        mini_app_id: impl Into<String>,
        mini_app_session_id: Option<String>,
        conversation_id: ConversationId,
        caller_actor_id: ActorId,
        service_actor_id: Option<ActorId>,
        mode: MiniAppServiceCallMode,
        mcp_tools: Vec<MiniAppMcpToolCapability>,
        dtmf_routes: Vec<MiniAppMcpDtmfRoute>,
        started_at_ms: i64,
    ) -> Result<Self, MiniAppServiceCallError> {
        let mini_app_id = mini_app_id.into();
        if !id.is_valid() || mini_app_id.trim().is_empty() || mini_app_id.len() > 200 {
            return Err(MiniAppServiceCallError::InvalidSession);
        }

        let mut tool_names = BTreeSet::new();
        for tool in &mcp_tools {
            if !tool.is_valid() {
                return Err(MiniAppServiceCallError::InvalidMcpTool);
            }
            if !tool_names.insert(tool.name.clone()) {
                return Err(MiniAppServiceCallError::DuplicateMcpTool(tool.name.clone()));
            }
        }

        let mut route_digits = BTreeSet::new();
        for route in &dtmf_routes {
            if !valid_dtmf(&route.digits)
                || !tool_names.contains(&route.tool_name)
                || !route_digits.insert(route.digits.clone())
            {
                return Err(MiniAppServiceCallError::InvalidDtmfRoute);
            }
        }

        Ok(Self {
            id,
            mini_app_id,
            mini_app_session_id,
            conversation_id,
            caller_actor_id,
            service_actor_id,
            mode,
            state: MiniAppServiceCallState::Connecting,
            mcp_tools,
            dtmf_routes,
            turns: Vec::new(),
            mcp_results: Vec::new(),
            started_at_ms,
            updated_at_ms: started_at_ms,
            ended_at_ms: None,
        })
    }

    pub fn activate(&mut self, now_ms: i64) -> Result<(), MiniAppServiceCallError> {
        if !matches!(
            self.state,
            MiniAppServiceCallState::Connecting | MiniAppServiceCallState::Held
        ) {
            return Err(MiniAppServiceCallError::InvalidState(self.state));
        }
        self.state = MiniAppServiceCallState::Active;
        self.updated_at_ms = now_ms;
        Ok(())
    }

    pub fn submit_input(
        &mut self,
        actor_id: ActorId,
        input: MiniAppServiceCallInput,
        now_ms: i64,
    ) -> Result<Vec<MiniAppServiceCallEffect>, MiniAppServiceCallError> {
        if self.state != MiniAppServiceCallState::Active {
            return Err(MiniAppServiceCallError::InvalidState(self.state));
        }
        if actor_id != self.caller_actor_id {
            return Err(MiniAppServiceCallError::ActorNotAllowed);
        }
        input.validate()?;

        let sequence = u64::try_from(self.turns.len())
            .unwrap_or(u64::MAX)
            .saturating_add(1);
        self.turns.push(MiniAppServiceCallTurn {
            sequence,
            actor_id: actor_id.clone(),
            input: input.clone(),
            created_at_ms: now_ms,
        });
        self.updated_at_ms = now_ms;

        let mut effects = Vec::new();
        match &input {
            MiniAppServiceCallInput::Dtmf { digits } => {
                effects.push(MiniAppServiceCallEffect::AppendConversationTranscript {
                    conversation_id: self.conversation_id.clone(),
                    actor_id: actor_id.clone(),
                    source: MiniAppServiceCallInputSource::Dtmf,
                    text: digits.clone(),
                    final_segment: true,
                    turn_sequence: sequence,
                });
            }
            MiniAppServiceCallInput::SpeechTranscript { text, is_final, .. } => {
                effects.push(MiniAppServiceCallEffect::AppendConversationTranscript {
                    conversation_id: self.conversation_id.clone(),
                    actor_id: actor_id.clone(),
                    source: MiniAppServiceCallInputSource::Speech,
                    text: text.clone(),
                    final_segment: *is_final,
                    turn_sequence: sequence,
                });
            }
            MiniAppServiceCallInput::ChatText { text } => {
                effects.push(MiniAppServiceCallEffect::AppendConversationTranscript {
                    conversation_id: self.conversation_id.clone(),
                    actor_id: actor_id.clone(),
                    source: MiniAppServiceCallInputSource::Chat,
                    text: text.clone(),
                    final_segment: true,
                    turn_sequence: sequence,
                });
            }
        }

        if let Some(digits) = input.normalized_dtmf_digits() {
            effects.push(self.route_dtmf_to_mcp(sequence, digits));
            return Ok(effects);
        }

        if input.requires_mcp_resolution() {
            if self.mcp_tools.is_empty() {
                effects.push(self.mcp_unavailable(
                    sequence,
                    "当前 MiniApp 没有暴露任何 MCP Tool，无法执行该需求".into(),
                ));
            } else {
                effects.push(MiniAppServiceCallEffect::ResolveMcpTool {
                    request: MiniAppMcpResolveRequest {
                        call_id: self.id.clone(),
                        mini_app_id: self.mini_app_id.clone(),
                        conversation_id: self.conversation_id.clone(),
                        turn_sequence: sequence,
                        input,
                        available_tools: self.mcp_tools.clone(),
                    },
                });
            }
        }
        Ok(effects)
    }

    pub fn apply_mcp_resolution(
        &self,
        turn_sequence: u64,
        resolution: MiniAppMcpToolResolution,
    ) -> Result<MiniAppServiceCallEffect, MiniAppServiceCallError> {
        if self.state != MiniAppServiceCallState::Active {
            return Err(MiniAppServiceCallError::InvalidState(self.state));
        }
        let turn = self
            .turns
            .iter()
            .find(|turn| turn.sequence == turn_sequence)
            .ok_or(MiniAppServiceCallError::TurnNotFound(turn_sequence))?;
        if !turn.input.requires_mcp_resolution() {
            return Err(MiniAppServiceCallError::TurnDoesNotRequireMcpResolution(
                turn_sequence,
            ));
        }

        match resolution {
            MiniAppMcpToolResolution::Invoke {
                tool_name,
                arguments,
            } => self.prepare_mcp_invocation(turn_sequence, tool_name, arguments),
            MiniAppMcpToolResolution::Unavailable { reason } => {
                let reason = reason.trim();
                if reason.is_empty() {
                    return Err(MiniAppServiceCallError::InvalidMcpResolution);
                }
                Ok(self.mcp_unavailable(turn_sequence, reason.to_string()))
            }
        }
    }

    pub fn prepare_mcp_invocation(
        &self,
        turn_sequence: u64,
        tool_name: impl Into<String>,
        arguments: MiniAppMcpArguments,
    ) -> Result<MiniAppServiceCallEffect, MiniAppServiceCallError> {
        let tool_name = tool_name.into();
        if !self.mcp_tools.iter().any(|tool| tool.name == tool_name) {
            return Err(MiniAppServiceCallError::McpToolUnavailable(tool_name));
        }
        Ok(MiniAppServiceCallEffect::InvokeMcpTool {
            request: MiniAppMcpInvocationRequest {
                call_id: self.id.clone(),
                mini_app_id: self.mini_app_id.clone(),
                conversation_id: self.conversation_id.clone(),
                turn_sequence,
                tool_name,
                arguments,
            },
        })
    }

    pub fn record_mcp_result(
        &mut self,
        result: MiniAppMcpInvocationResult,
    ) -> Result<(), MiniAppServiceCallError> {
        if matches!(
            self.state,
            MiniAppServiceCallState::Ended | MiniAppServiceCallState::Failed
        ) {
            return Err(MiniAppServiceCallError::InvalidState(self.state));
        }
        if result.invocation_id.trim().is_empty()
            || result.tool_name.trim().is_empty()
            || !self
                .mcp_tools
                .iter()
                .any(|tool| tool.name == result.tool_name)
        {
            return Err(MiniAppServiceCallError::InvalidMcpResult);
        }
        self.updated_at_ms = result.completed_at_ms;
        self.mcp_results.push(result);
        Ok(())
    }

    pub fn end(&mut self, now_ms: i64) -> Result<(), MiniAppServiceCallError> {
        if matches!(
            self.state,
            MiniAppServiceCallState::Ended | MiniAppServiceCallState::Failed
        ) {
            return Err(MiniAppServiceCallError::InvalidState(self.state));
        }
        self.state = MiniAppServiceCallState::Ended;
        self.updated_at_ms = now_ms;
        self.ended_at_ms = Some(now_ms);
        Ok(())
    }

    fn route_dtmf_to_mcp(&self, turn_sequence: u64, digits: &str) -> MiniAppServiceCallEffect {
        if let Some(route) = self.dtmf_routes.iter().find(|route| route.digits == digits) {
            return MiniAppServiceCallEffect::InvokeMcpTool {
                request: MiniAppMcpInvocationRequest {
                    call_id: self.id.clone(),
                    mini_app_id: self.mini_app_id.clone(),
                    conversation_id: self.conversation_id.clone(),
                    turn_sequence,
                    tool_name: route.tool_name.clone(),
                    arguments: route.arguments.clone(),
                },
            };
        }
        self.mcp_unavailable(
            turn_sequence,
            format!("当前 MiniApp 没有为数字 {digits} 配置可调用的 MCP Tool"),
        )
    }

    fn mcp_unavailable(&self, turn_sequence: u64, reason: String) -> MiniAppServiceCallEffect {
        MiniAppServiceCallEffect::McpCapabilityUnavailable {
            call_id: self.id.clone(),
            mini_app_id: self.mini_app_id.clone(),
            turn_sequence,
            reason,
        }
    }
}

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum MiniAppServiceCallError {
    #[error("MiniApp service call session is invalid")]
    InvalidSession,
    #[error("MiniApp service call is not writable in state {0:?}")]
    InvalidState(MiniAppServiceCallState),
    #[error("actor is not allowed to submit input to this MiniApp service call")]
    ActorNotAllowed,
    #[error("DTMF input is invalid")]
    InvalidDtmf,
    #[error("transcript input is invalid")]
    InvalidTranscript,
    #[error("speech confidence must be between 0 and 10000 basis points")]
    InvalidConfidence,
    #[error("MiniApp MCP Tool is invalid")]
    InvalidMcpTool,
    #[error("MiniApp MCP Tool is duplicated: {0}")]
    DuplicateMcpTool(String),
    #[error("MiniApp DTMF route is invalid or references a Tool not exposed by this MiniApp")]
    InvalidDtmfRoute,
    #[error("MiniApp MCP Tool is not available for this MiniApp: {0}")]
    McpToolUnavailable(String),
    #[error("service call turn does not exist: {0}")]
    TurnNotFound(u64),
    #[error("service call turn does not require MCP semantic resolution: {0}")]
    TurnDoesNotRequireMcpResolution(u64),
    #[error("MCP Tool resolution is invalid")]
    InvalidMcpResolution,
    #[error("MiniApp MCP invocation result is invalid")]
    InvalidMcpResult,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn tool(name: &str, description: &str, read_only: bool) -> MiniAppMcpToolCapability {
        MiniAppMcpToolCapability {
            name: name.into(),
            title: None,
            description: Some(description.into()),
            input_schema: Some(json!({"type": "object"})),
            annotations: MiniAppMcpToolAnnotations {
                read_only_hint: read_only,
                destructive_hint: false,
                open_world_hint: false,
            },
        }
    }

    fn session() -> MiniAppServiceCallSession {
        MiniAppServiceCallSession::new(
            MiniAppServiceCallId::new("svc-call:1"),
            "miniapp:carrier",
            Some("miniapp-session:1".into()),
            ConversationId("conversation:carrier".into()),
            ActorId("actor:user".into()),
            Some(ActorId("actor:carrier-service".into())),
            MiniAppServiceCallMode::Hybrid,
            vec![
                tool("query_allowance", "查询套餐余量", true),
                tool("change_plan", "变更套餐", false),
            ],
            vec![
                MiniAppMcpDtmfRoute {
                    digits: "1".into(),
                    tool_name: "query_allowance".into(),
                    arguments: BTreeMap::new(),
                },
                MiniAppMcpDtmfRoute {
                    digits: "2".into(),
                    tool_name: "change_plan".into(),
                    arguments: BTreeMap::new(),
                },
            ],
            100,
        )
        .expect("valid session")
    }

    #[test]
    fn dtmf_routes_directly_to_mcp_tool_call() {
        let mut call = session();
        call.activate(110).expect("activate");
        let effects = call
            .submit_input(
                ActorId("actor:user".into()),
                MiniAppServiceCallInput::Dtmf { digits: "1".into() },
                120,
            )
            .expect("submit dtmf");
        assert_eq!(effects.len(), 2);
        assert!(effects.iter().any(|effect| matches!(
            effect,
            MiniAppServiceCallEffect::InvokeMcpTool { request }
                if request.tool_name == "query_allowance"
        )));
        assert!(!effects
            .iter()
            .any(|effect| matches!(effect, MiniAppServiceCallEffect::ResolveMcpTool { .. })));
    }

    #[test]
    fn chat_numeric_input_uses_same_dtmf_mcp_route() {
        let mut call = session();
        call.activate(110).expect("activate");
        let effects = call
            .submit_input(
                ActorId("actor:user".into()),
                MiniAppServiceCallInput::ChatText { text: "2".into() },
                120,
            )
            .expect("submit chat digit");
        assert!(effects.iter().any(|effect| matches!(
            effect,
            MiniAppServiceCallEffect::InvokeMcpTool { request }
                if request.tool_name == "change_plan"
        )));
    }

    #[test]
    fn final_speech_is_transcribed_and_sent_to_mcp_tool_resolution() {
        let mut call = session();
        call.activate(110).expect("activate");
        let effects = call
            .submit_input(
                ActorId("actor:user".into()),
                MiniAppServiceCallInput::SpeechTranscript {
                    text: "帮我查一下本月套餐余量".into(),
                    is_final: true,
                    confidence_bps: Some(9_500),
                },
                120,
            )
            .expect("submit speech");
        assert_eq!(effects.len(), 2);
        assert!(effects.iter().any(|effect| matches!(
            effect,
            MiniAppServiceCallEffect::AppendConversationTranscript {
                source: MiniAppServiceCallInputSource::Speech,
                final_segment: true,
                ..
            }
        )));
        assert!(effects.iter().any(|effect| matches!(
            effect,
            MiniAppServiceCallEffect::ResolveMcpTool { request }
                if request.mini_app_id == "miniapp:carrier"
                    && request.available_tools.len() == 2
        )));
    }

    #[test]
    fn interim_speech_is_visible_but_not_executed() {
        let mut call = session();
        call.activate(110).expect("activate");
        let effects = call
            .submit_input(
                ActorId("actor:user".into()),
                MiniAppServiceCallInput::SpeechTranscript {
                    text: "帮我查一下".into(),
                    is_final: false,
                    confidence_bps: None,
                },
                120,
            )
            .expect("submit speech");
        assert_eq!(effects.len(), 1);
        assert!(matches!(
            &effects[0],
            MiniAppServiceCallEffect::AppendConversationTranscript {
                final_segment: false,
                ..
            }
        ));
    }

    #[test]
    fn chat_text_uses_same_mcp_resolution_pipeline_as_final_speech() {
        let mut call = session();
        call.activate(110).expect("activate");
        let effects = call
            .submit_input(
                ActorId("actor:user".into()),
                MiniAppServiceCallInput::ChatText {
                    text: "把套餐改成最低月租".into(),
                },
                120,
            )
            .expect("submit chat text");
        assert!(effects
            .iter()
            .any(|effect| matches!(effect, MiniAppServiceCallEffect::ResolveMcpTool { .. })));
    }

    #[test]
    fn resolver_cannot_select_tool_not_exposed_by_miniapp() {
        let mut call = session();
        call.activate(110).expect("activate");
        call.submit_input(
            ActorId("actor:user".into()),
            MiniAppServiceCallInput::ChatText {
                text: "帮我注销账户".into(),
            },
            120,
        )
        .expect("submit chat text");
        let error = call
            .apply_mcp_resolution(
                1,
                MiniAppMcpToolResolution::Invoke {
                    tool_name: "delete_account".into(),
                    arguments: BTreeMap::new(),
                },
            )
            .expect_err("unexposed tool must be rejected");
        assert_eq!(
            error,
            MiniAppServiceCallError::McpToolUnavailable("delete_account".into())
        );
    }

    #[test]
    fn no_matching_mcp_tool_is_reported_without_execution() {
        let mut call = session();
        call.activate(110).expect("activate");
        call.submit_input(
            ActorId("actor:user".into()),
            MiniAppServiceCallInput::ChatText {
                text: "帮我办理一个当前没有的业务".into(),
            },
            120,
        )
        .expect("submit chat text");
        let effect = call
            .apply_mcp_resolution(
                1,
                MiniAppMcpToolResolution::Unavailable {
                    reason: "当前 MiniApp 没有对应 MCP Tool".into(),
                },
            )
            .expect("unavailable is a valid resolution");
        assert!(matches!(
            effect,
            MiniAppServiceCallEffect::McpCapabilityUnavailable { .. }
        ));
    }

    #[test]
    fn unmapped_dtmf_returns_unavailable_without_direct_execution() {
        let mut call = session();
        call.activate(110).expect("activate");
        let effects = call
            .submit_input(
                ActorId("actor:user".into()),
                MiniAppServiceCallInput::Dtmf { digits: "9".into() },
                120,
            )
            .expect("submit dtmf");
        assert!(effects.iter().any(|effect| matches!(
            effect,
            MiniAppServiceCallEffect::McpCapabilityUnavailable { .. }
        )));
        assert!(!effects
            .iter()
            .any(|effect| matches!(effect, MiniAppServiceCallEffect::InvokeMcpTool { .. })));
    }

    #[test]
    fn ended_call_rejects_new_input() {
        let mut call = session();
        call.activate(110).expect("activate");
        call.end(120).expect("end");
        let error = call
            .submit_input(
                ActorId("actor:user".into()),
                MiniAppServiceCallInput::ChatText {
                    text: "继续".into(),
                },
                130,
            )
            .expect_err("ended call must reject input");
        assert_eq!(
            error,
            MiniAppServiceCallError::InvalidState(MiniAppServiceCallState::Ended)
        );
    }

    #[test]
    fn mcp_results_are_auditable_on_the_call_session() {
        let mut call = session();
        call.activate(110).expect("activate");
        call.record_mcp_result(MiniAppMcpInvocationResult {
            invocation_id: "mcp-call:1".into(),
            tool_name: "query_allowance".into(),
            success: true,
            structured_content: Some(json!({"remainingGb": 20})),
            error: None,
            spoken_response: Some("本月还剩 20GB".into()),
            display_response: Some("剩余流量 20GB".into()),
            completed_at_ms: 130,
        })
        .expect("record result");
        assert_eq!(call.mcp_results.len(), 1);
        assert_eq!(call.mcp_results[0].tool_name, "query_allowance");
    }

    #[test]
    fn dtmf_route_cannot_reference_unexposed_mcp_tool() {
        let error = MiniAppServiceCallSession::new(
            MiniAppServiceCallId::new("svc-call:1"),
            "miniapp:carrier",
            Some("miniapp-session:1".into()),
            ConversationId("conversation:carrier".into()),
            ActorId("actor:user".into()),
            None,
            MiniAppServiceCallMode::Text,
            vec![tool("query_allowance", "查询套餐余量", true)],
            vec![MiniAppMcpDtmfRoute {
                digits: "1".into(),
                tool_name: "missing_tool".into(),
                arguments: BTreeMap::new(),
            }],
            100,
        )
        .expect_err("route to unexposed tool must fail");
        assert_eq!(error, MiniAppServiceCallError::InvalidDtmfRoute);
    }
}
