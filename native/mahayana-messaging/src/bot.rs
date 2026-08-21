use crate::actor::ActorId;
use crate::conversation::ConversationId;
use crate::message::{FormattedText, MessageContent, ReplyMarkup};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use thiserror::Error;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BotCommand {
    pub command: String,
    pub description: String,
    pub scopes: BTreeSet<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BotProfile {
    pub actor_id: ActorId,
    pub description: String,
    pub about: String,
    pub commands: Vec<BotCommand>,
    pub inline_mode_enabled: bool,
    pub inline_placeholder: Option<String>,
    pub groups_allowed: bool,
    pub privacy_mode: bool,
    pub mini_app_id: Option<String>,
    pub payment_provider_ids: Vec<String>,
    pub business_mode: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BotInvocation {
    pub id: String,
    pub bot_id: ActorId,
    pub sender_id: ActorId,
    pub conversation_id: ConversationId,
    pub command: Option<String>,
    pub text: FormattedText,
    pub reply_to_message_id: Option<String>,
    pub metadata: BTreeMap<String, String>,
    pub created_at_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BotReply {
    pub invocation_id: String,
    pub content: MessageContent,
    pub reply_markup: Option<ReplyMarkup>,
    pub silent: bool,
    pub protect_content: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InlineQuery {
    pub id: String,
    pub bot_id: ActorId,
    pub sender_id: ActorId,
    pub query: String,
    pub offset: Option<String>,
    pub conversation_id: Option<ConversationId>,
    pub created_at_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InlineResult {
    pub id: String,
    pub title: String,
    pub description: Option<String>,
    pub content: MessageContent,
    pub reply_markup: Option<ReplyMarkup>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InlineQueryResult {
    pub query_id: String,
    pub results: Vec<InlineResult>,
    pub next_offset: Option<String>,
    pub cache_seconds: u32,
    pub personal: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum BotExecutionState {
    Queued,
    Running,
    WaitingForApproval,
    Completed,
    Failed,
    Cancelled,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BotExecution {
    pub id: String,
    pub bot_id: ActorId,
    pub invocation_id: String,
    pub state: BotExecutionState,
    pub started_at_ms: Option<i64>,
    pub finished_at_ms: Option<i64>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BotRegistry {
    pub bots: BTreeMap<ActorId, BotProfile>,
    pub executions: BTreeMap<String, BotExecution>,
}

impl BotRegistry {
    pub fn register(&mut self, profile: BotProfile) -> Result<(), BotError> {
        if !profile.actor_id.is_valid() {
            return Err(BotError::InvalidBotId);
        }
        for command in &profile.commands {
            validate_command(command)?;
        }
        self.bots.insert(profile.actor_id.clone(), profile);
        Ok(())
    }

    pub fn begin_execution(
        &mut self,
        invocation: &BotInvocation,
        now_ms: i64,
    ) -> Result<BotExecution, BotError> {
        let profile = self
            .bots
            .get(&invocation.bot_id)
            .ok_or_else(|| BotError::BotNotFound(invocation.bot_id.clone()))?;
        if invocation.command.is_some()
            && profile.privacy_mode
            && invocation.text.text.trim().is_empty()
        {
            return Err(BotError::EmptyInvocation);
        }
        let execution = BotExecution {
            id: format!("bot-execution:{}", invocation.id),
            bot_id: invocation.bot_id.clone(),
            invocation_id: invocation.id.clone(),
            state: BotExecutionState::Running,
            started_at_ms: Some(now_ms),
            finished_at_ms: None,
            error: None,
        };
        self.executions
            .insert(execution.id.clone(), execution.clone());
        Ok(execution)
    }

    pub fn finish_execution(
        &mut self,
        execution_id: &str,
        success: bool,
        now_ms: i64,
        error: Option<String>,
    ) -> Result<(), BotError> {
        let execution = self
            .executions
            .get_mut(execution_id)
            .ok_or_else(|| BotError::ExecutionNotFound(execution_id.to_string()))?;
        execution.state = if success {
            BotExecutionState::Completed
        } else {
            BotExecutionState::Failed
        };
        execution.finished_at_ms = Some(now_ms);
        execution.error = error;
        Ok(())
    }
}

fn validate_command(command: &BotCommand) -> Result<(), BotError> {
    let value = command.command.trim_start_matches('/');
    if value.is_empty()
        || value.len() > 32
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_')
    {
        return Err(BotError::InvalidCommand(command.command.clone()));
    }
    Ok(())
}

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum BotError {
    #[error("bot identifier is invalid")]
    InvalidBotId,
    #[error("bot {0:?} was not found")]
    BotNotFound(ActorId),
    #[error("bot command {0} is invalid")]
    InvalidCommand(String),
    #[error("bot invocation is empty")]
    EmptyInvocation,
    #[error("bot execution {0} was not found")]
    ExecutionNotFound(String),
}
