use crate::{ModelError, ModelEvent, ModelRequest, ModelRuntime, ModelUsage, SharedModelEventSink};
use async_trait::async_trait;
use mahayana_core::ModelProviderMode;
use serde_json::{Value, json};
use std::io::{BufRead, BufReader};
use std::sync::Arc;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum ResponsesWireApi {
    #[default]
    Responses,
    ChatCompletions,
    AnthropicMessages,
}

#[derive(Debug, Clone)]
pub struct ResponsesModelConfig {
    pub base_url: String,
    pub default_model: String,
    pub bearer_token: Option<String>,
    pub provider_mode: ModelProviderMode,
    pub wire_api: ResponsesWireApi,
}

impl ResponsesModelConfig {
    pub fn validate(&self) -> Result<(), ModelError> {
        if self.default_model.trim().is_empty() {
            return Err(ModelError::InvalidRequest(
                "default model must not be empty".into(),
            ));
        }
        if !self.base_url.starts_with("https://")
            && !matches!(self.provider_mode, ModelProviderMode::LocalLoopback)
        {
            return Err(ModelError::InvalidRequest(
                "remote model endpoints must use HTTPS".into(),
            ));
        }
        if self
            .bearer_token
            .as_deref()
            .is_some_and(|token| token.trim().is_empty() || token.contains(['\r', '\n']))
        {
            return Err(ModelError::InvalidRequest(
                "model credential contains invalid header characters".into(),
            ));
        }
        Ok(())
    }
}

/// Provider-neutral Responses API implementation of Mahayana's model boundary.
///
/// This component performs model inference only. It does not host an Agent,
/// execute tools, own workspace state, or make policy decisions; those belong
/// to `mahayana-native-engine` and the sovereign kernel.
pub struct ResponsesModelRuntime {
    config: ResponsesModelConfig,
    credential_resolver: Option<ModelCredentialResolver>,
}

/// Resolves the current product-account bearer token at inference time. A
/// long-lived desktop Host must not retain the previous account's credential
/// after logout or account switching.
pub type ModelCredentialResolver =
    Arc<dyn Fn() -> Result<Option<String>, ModelError> + Send + Sync>;

impl ResponsesModelRuntime {
    pub fn new(config: ResponsesModelConfig) -> Result<Self, ModelError> {
        config.validate()?;
        Ok(Self {
            config,
            credential_resolver: None,
        })
    }

    pub fn with_credential_resolver(mut self, resolver: ModelCredentialResolver) -> Self {
        self.credential_resolver = Some(resolver);
        self
    }
}

#[async_trait]
impl ModelRuntime for ResponsesModelRuntime {
    async fn infer(
        &self,
        mut request: ModelRequest,
        events: SharedModelEventSink,
    ) -> Result<(), ModelError> {
        if request.model.trim().is_empty() {
            request.model = self.config.default_model.clone();
        }
        if request.model.trim().is_empty() {
            return Err(ModelError::InvalidRequest("model must not be empty".into()));
        }

        let mut config = self.config.clone();
        if let Some(resolver) = self.credential_resolver.as_ref() {
            config.bearer_token = resolver()?;
        }
        let events_for_request = Arc::clone(&events);
        let (payload, streamed_text) = tokio::task::spawn_blocking(move || {
            request_response(&config, request, events_for_request)
        })
        .await
        .map_err(|error| ModelError::Inference(format!("model task failed: {error}")))??;

        // SSE deltas have already reached the Agent event sink. Only emit the
        // final text for JSON/fallback endpoints to avoid duplicating replies.
        if !streamed_text {
            if let Some(text) = extract_output_text(&payload) {
                events.emit(ModelEvent::OutputTextDelta(text))?;
            }
        }
        if let Some(usage) = extract_usage(&payload) {
            events.emit(ModelEvent::Usage(usage))?;
        }
        events.emit(ModelEvent::Completed { output: payload })
    }

    fn provider_mode(&self) -> ModelProviderMode {
        self.config.provider_mode
    }
}

fn request_response(
    config: &ResponsesModelConfig,
    request: ModelRequest,
    events: SharedModelEventSink,
) -> Result<(Value, bool), ModelError> {
    if matches!(
        config.provider_mode,
        ModelProviderMode::UserConfiguredRemote
    ) && config.bearer_token.is_none()
    {
        return Err(ModelError::InvalidRequest(
            "selected model provider credential is not configured".into(),
        ));
    }
    let (endpoint, body) = match config.wire_api {
        ResponsesWireApi::Responses => {
            let endpoint = if config.base_url.ends_with("/responses") {
                config.base_url.clone()
            } else {
                format!("{}/responses", config.base_url.trim_end_matches('/'))
            };
            let mut body =
                json!({ "model": request.model, "input": request.input, "stream": true });
            for key in [
                "tools",
                "tool_choice",
                "parallel_tool_calls",
                "instructions",
                "reasoning",
                "text",
                "temperature",
                "max_output_tokens",
            ] {
                if let Some(value) = request.metadata.get(key) {
                    body[key] = value.clone();
                }
            }
            (endpoint, body)
        }
        ResponsesWireApi::ChatCompletions => {
            let endpoint = if config.base_url.ends_with("/chat/completions") {
                config.base_url.clone()
            } else {
                format!("{}/chat/completions", config.base_url.trim_end_matches('/'))
            };
            let mut messages = chat_messages(&request.input);
            if let Some(instructions) = request.metadata.get("instructions").and_then(Value::as_str)
                && !instructions.trim().is_empty()
            {
                messages.insert(0, json!({"role":"system", "content": instructions}));
            }
            let mut body = json!({ "model": request.model, "messages": messages, "stream": false });
            if let Some(tools) = request.metadata.get("tools").and_then(Value::as_array) {
                body["tools"] = Value::Array(tools.iter().filter_map(chat_tool).collect());
            }
            for key in ["tool_choice", "parallel_tool_calls", "temperature"] {
                if let Some(value) = request.metadata.get(key) {
                    body[key] = value.clone();
                }
            }
            if let Some(value) = request.metadata.get("max_output_tokens") {
                body["max_tokens"] = value.clone();
            }
            (endpoint, body)
        }
        ResponsesWireApi::AnthropicMessages => {
            let endpoint = if config.base_url.ends_with("/messages") {
                config.base_url.clone()
            } else {
                format!("{}/messages", config.base_url.trim_end_matches('/'))
            };
            let mut body = json!({
                "model": request.model,
                "messages": anthropic_messages(&request.input),
                "max_tokens": request.metadata.get("max_output_tokens").cloned().unwrap_or_else(|| json!(4096)),
            });
            let system = anthropic_system(&request.input, request.metadata.get("instructions"));
            if !system.is_empty() {
                body["system"] = json!(system);
            }
            if let Some(tools) = request.metadata.get("tools").and_then(Value::as_array) {
                body["tools"] = Value::Array(tools.iter().filter_map(anthropic_tool).collect());
            }
            if let Some(value) = request.metadata.get("temperature") {
                body["temperature"] = value.clone();
            }
            (endpoint, body)
        }
    };

    let accept = if matches!(config.wire_api, ResponsesWireApi::Responses) {
        "text/event-stream, application/json"
    } else {
        "application/json"
    };
    let mut http = ureq::post(&endpoint).set("Accept", accept);
    if let Some(token) = config.bearer_token.as_deref() {
        http = match config.wire_api {
            ResponsesWireApi::AnthropicMessages => http
                .set("x-api-key", token)
                .set("anthropic-version", "2023-06-01"),
            ResponsesWireApi::Responses | ResponsesWireApi::ChatCompletions => {
                http.set("Authorization", &format!("Bearer {token}"))
            }
        };
    }
    let response = http.send_json(body).map_err(redacted_http_error)?;
    if matches!(config.wire_api, ResponsesWireApi::Responses)
        && response
            .header("Content-Type")
            .unwrap_or_default()
            .to_ascii_lowercase()
            .contains("text/event-stream")
    {
        return request_stream(response, events);
    }
    let payload: Value = response
        .into_json()
        .map_err(|_| ModelError::Inference("model endpoint returned invalid JSON".into()))?;
    if let Some(error) = payload.get("error") {
        let message = error
            .get("message")
            .and_then(Value::as_str)
            .unwrap_or("model endpoint returned an error");
        return Err(ModelError::Inference(message.to_string()));
    }
    Ok((
        match config.wire_api {
            ResponsesWireApi::Responses => payload,
            ResponsesWireApi::ChatCompletions => normalize_chat_payload(payload),
            ResponsesWireApi::AnthropicMessages => normalize_anthropic_payload(payload),
        },
        false,
    ))
}

fn request_stream(
    response: ureq::Response,
    events: SharedModelEventSink,
) -> Result<(Value, bool), ModelError> {
    let mut reader = BufReader::new(response.into_reader());
    let mut data = String::new();
    let mut streamed_text = false;
    let mut accumulated_text = String::new();
    let mut final_payload = None;

    loop {
        let mut line = String::new();
        let read = reader
            .read_line(&mut line)
            .map_err(|error| ModelError::Inference(format!("model stream read failed: {error}")))?;
        if read == 0 {
            consume_sse_event(
                &data,
                &events,
                &mut accumulated_text,
                &mut streamed_text,
                &mut final_payload,
            )?;
            break;
        }
        let line = line.trim_end_matches(['\r', '\n']);
        if line.is_empty() {
            consume_sse_event(
                &data,
                &events,
                &mut accumulated_text,
                &mut streamed_text,
                &mut final_payload,
            )?;
            data.clear();
            continue;
        }
        if line.starts_with(':') {
            continue;
        }
        if let Some(value) = line.strip_prefix("data:") {
            if !data.is_empty() {
                data.push('\n');
            }
            data.push_str(value.trim_start());
        }
    }

    let payload = final_payload.unwrap_or_else(|| {
        json!({
            "id": "resp_local",
            "object": "response",
            "status": "completed",
            "output": if accumulated_text.is_empty() {
                Vec::<Value>::new()
            } else {
                vec![json!({
                    "type": "message",
                    "role": "assistant",
                    "content": [{"type": "output_text", "text": accumulated_text}],
                })]
            },
        })
    });
    validate_response_payload(&payload)?;
    Ok((payload, streamed_text))
}

fn consume_sse_event(
    data: &str,
    events: &SharedModelEventSink,
    accumulated_text: &mut String,
    streamed_text: &mut bool,
    final_payload: &mut Option<Value>,
) -> Result<(), ModelError> {
    let data = data.trim();
    if data.is_empty() || data == "[DONE]" {
        return Ok(());
    }
    let payload: Value = serde_json::from_str(data).map_err(|error| {
        ModelError::Inference(format!("model stream returned invalid JSON: {error}"))
    })?;
    let event_type = payload
        .get("type")
        .and_then(Value::as_str)
        .unwrap_or_default();
    match event_type {
        "response.output_text.delta" => {
            if let Some(delta) = payload.get("delta").and_then(Value::as_str)
                && !delta.is_empty()
            {
                accumulated_text.push_str(delta);
                *streamed_text = true;
                events.emit(ModelEvent::OutputTextDelta(delta.to_string()))?;
            }
        }
        "response.completed" => {
            *final_payload = payload.get("response").cloned().or(Some(payload));
        }
        "response.failed" | "response.incomplete" => {
            let message = payload
                .pointer("/response/error/message")
                .or_else(|| payload.pointer("/error/message"))
                .and_then(Value::as_str)
                .unwrap_or("model endpoint returned an incomplete response");
            return Err(ModelError::Inference(message.to_string()));
        }
        _ => {
            // Compatibility with an upstream that sends chat-completions
            // chunks while advertising an event-stream response.
            if let Some(delta) = payload
                .pointer("/choices/0/delta/content")
                .and_then(Value::as_str)
                && !delta.is_empty()
            {
                accumulated_text.push_str(delta);
                *streamed_text = true;
                events.emit(ModelEvent::OutputTextDelta(delta.to_string()))?;
            }
        }
    }
    Ok(())
}

fn validate_response_payload(payload: &Value) -> Result<(), ModelError> {
    if let Some(error) = payload.get("error") {
        let message = error
            .get("message")
            .and_then(Value::as_str)
            .unwrap_or("model endpoint returned an error");
        return Err(ModelError::Inference(message.to_string()));
    }
    Ok(())
}

fn anthropic_messages(input: &Value) -> Vec<Value> {
    let mut messages: Vec<Value> = Vec::new();
    for item in input.as_array().into_iter().flatten() {
        if let Some(role) = item.get("role").and_then(Value::as_str) {
            if role == "system" {
                continue;
            }
            let content = item.get("content").cloned().unwrap_or_else(|| json!(""));
            let content = match content {
                Value::String(text) => json!([{"type":"text", "text":text}]),
                Value::Array(mut parts) => {
                    for part in &mut parts {
                        if matches!(
                            part.get("type").and_then(Value::as_str),
                            Some("input_text" | "output_text")
                        ) {
                            part["type"] = json!("text");
                        }
                    }
                    Value::Array(parts)
                }
                other => json!([{"type":"text", "text":other.to_string()}]),
            };
            messages.push(json!({"role": role, "content": content}));
            continue;
        }
        match item.get("type").and_then(Value::as_str) {
            Some("function_call") | Some("tool_call") => {
                let arguments = item
                    .get("arguments")
                    .or_else(|| item.pointer("/function/arguments"))
                    .cloned()
                    .unwrap_or_else(|| json!({}));
                let arguments = arguments
                    .as_str()
                    .and_then(|value| serde_json::from_str(value).ok())
                    .unwrap_or(arguments);
                let block = json!({
                    "type": "tool_use",
                    "id": item.get("call_id").or_else(|| item.get("id")).cloned().unwrap_or_else(|| json!("call")),
                    "name": item.get("name").or_else(|| item.pointer("/function/name")).cloned().unwrap_or_else(|| json!("tool")),
                    "input": arguments,
                });
                append_anthropic_content(&mut messages, "assistant", block);
            }
            Some("function_call_output") => {
                let content = item
                    .get("output")
                    .map(|value| match value {
                        Value::String(text) => text.clone(),
                        other => serde_json::to_string(other).unwrap_or_else(|_| "null".into()),
                    })
                    .unwrap_or_else(|| "null".into());
                let block = json!({
                    "type": "tool_result",
                    "tool_use_id": item.get("call_id").cloned().unwrap_or_else(|| json!("call")),
                    "content": content,
                });
                append_anthropic_content(&mut messages, "user", block);
            }
            _ => {}
        }
    }
    messages
}

fn anthropic_system(input: &Value, instructions: Option<&Value>) -> String {
    let mut sections = Vec::new();
    if let Some(value) = instructions.and_then(Value::as_str)
        && !value.trim().is_empty()
    {
        sections.push(value.trim().to_owned());
    }
    for item in input.as_array().into_iter().flatten() {
        if item.get("role").and_then(Value::as_str) != Some("system") {
            continue;
        }
        if let Some(value) = item.get("content").and_then(Value::as_str)
            && !value.trim().is_empty()
        {
            sections.push(value.trim().to_owned());
        }
    }
    sections.join("\n\n")
}

fn append_anthropic_content(messages: &mut Vec<Value>, role: &str, block: Value) {
    if let Some(content) = messages
        .last_mut()
        .filter(|message| message.get("role").and_then(Value::as_str) == Some(role))
        .and_then(|message| message.get_mut("content"))
        .and_then(Value::as_array_mut)
    {
        content.push(block);
    } else {
        messages.push(json!({"role": role, "content": [block]}));
    }
}

fn anthropic_tool(tool: &Value) -> Option<Value> {
    let name = tool.get("name").and_then(Value::as_str)?;
    Some(json!({
        "name": name,
        "description": tool.get("description").cloned().unwrap_or_else(|| json!("")),
        "input_schema": tool.get("parameters").cloned().unwrap_or_else(|| json!({"type":"object","properties":{}})),
    }))
}

fn normalize_anthropic_payload(payload: Value) -> Value {
    let mut output = Vec::new();
    for block in payload
        .get("content")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        match block.get("type").and_then(Value::as_str) {
            Some("text") => output.push(json!({
                "type": "message",
                "role": "assistant",
                "content": [{"type":"output_text", "text":block.get("text").cloned().unwrap_or_else(|| json!(""))}],
            })),
            Some("tool_use") => output.push(json!({
                "type": "function_call",
                "call_id": block.get("id").cloned().unwrap_or_else(|| json!("call")),
                "name": block.get("name").cloned().unwrap_or_else(|| json!("tool")),
                "arguments": serde_json::to_string(block.get("input").unwrap_or(&Value::Null)).unwrap_or_else(|_| "{}".into()),
            })),
            _ => {}
        }
    }
    let usage = payload.get("usage").cloned().unwrap_or(Value::Null);
    let uncached_input_tokens = usage
        .get("input_tokens")
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let cache_creation_input_tokens = usage
        .get("cache_creation_input_tokens")
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let input_tokens = uncached_input_tokens.saturating_add(cache_creation_input_tokens);
    let output_tokens = usage
        .get("output_tokens")
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let cached_input_tokens = usage
        .get("cache_read_input_tokens")
        .and_then(Value::as_u64)
        .unwrap_or(0);
    json!({
        "id": payload.get("id").cloned().unwrap_or(Value::Null),
        "output": output,
        "usage": {
            "input_tokens": input_tokens,
            "cached_input_tokens": cached_input_tokens,
            "output_tokens": output_tokens,
            "total_tokens": input_tokens.saturating_add(cached_input_tokens).saturating_add(output_tokens),
        },
    })
}

fn chat_messages(input: &Value) -> Vec<Value> {
    let mut messages = Vec::new();
    for item in input.as_array().into_iter().flatten() {
        if let Some(role) = item.get("role").and_then(Value::as_str) {
            let content = item
                .get("content")
                .cloned()
                .unwrap_or(Value::String(String::new()));
            let content = if let Some(parts) = content.as_array() {
                Value::String(
                    parts
                        .iter()
                        .filter_map(|part| part.get("text").and_then(Value::as_str))
                        .collect::<Vec<_>>()
                        .join(""),
                )
            } else {
                content
            };
            messages.push(json!({"role": role, "content": content}));
            continue;
        }
        match item.get("type").and_then(Value::as_str) {
            Some("function_call") | Some("tool_call") => {
                let tool_call = json!({
                    "id": item.get("call_id").or_else(|| item.get("id")).cloned().unwrap_or_else(|| json!("call")),
                    "type": "function",
                    "function": {
                        "name": item.get("name").or_else(|| item.pointer("/function/name")).cloned().unwrap_or_else(|| json!("tool")),
                        "arguments": item.get("arguments").or_else(|| item.pointer("/function/arguments")).cloned().unwrap_or_else(|| json!("{}")),
                    }
                });
                if let Some(calls) = messages
                    .last_mut()
                    .filter(|message| {
                        message.get("role").and_then(Value::as_str) == Some("assistant")
                    })
                    .and_then(|message| message.get_mut("tool_calls"))
                    .and_then(Value::as_array_mut)
                {
                    calls.push(tool_call);
                } else {
                    messages.push(json!({
                        "role": "assistant",
                        "content": Value::Null,
                        "tool_calls": [tool_call]
                    }));
                }
            }
            Some("function_call_output") => messages.push(json!({
                "role": "tool",
                "tool_call_id": item.get("call_id").cloned().unwrap_or_else(|| json!("call")),
                "content": item.get("output").cloned().unwrap_or_else(|| json!("null")),
            })),
            _ => {}
        }
    }
    messages
}

fn chat_tool(tool: &Value) -> Option<Value> {
    let name = tool.get("name").and_then(Value::as_str)?;
    Some(json!({
        "type": "function",
        "function": {
            "name": name,
            "description": tool.get("description").cloned().unwrap_or_else(|| json!("")),
            "parameters": tool.get("parameters").cloned().unwrap_or_else(|| json!({"type":"object","properties":{}})),
        }
    }))
}

fn normalize_chat_payload(payload: Value) -> Value {
    let message = payload
        .pointer("/choices/0/message")
        .cloned()
        .unwrap_or(Value::Null);
    let mut output = Vec::new();
    if let Some(text) = message.get("content").and_then(Value::as_str)
        && !text.is_empty()
    {
        output.push(json!({"type":"message", "role":"assistant", "content":[{"type":"output_text", "text":text}]}));
    }
    for call in message
        .get("tool_calls")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        output.push(json!({
            "type": "function_call",
            "call_id": call.get("id").cloned().unwrap_or_else(|| json!("call")),
            "name": call.pointer("/function/name").cloned().unwrap_or_else(|| json!("tool")),
            "arguments": call.pointer("/function/arguments").cloned().unwrap_or_else(|| json!("{}")),
        }));
    }
    json!({
        "id": payload.get("id").cloned().unwrap_or(Value::Null),
        "output": output,
        "usage": payload.get("usage").cloned().unwrap_or(Value::Null),
    })
}

fn redacted_http_error(error: ureq::Error) -> ModelError {
    match error {
        ureq::Error::Status(status, response) => {
            let payload = response.into_json::<Value>().unwrap_or(Value::Null);
            let message = payload
                .pointer("/error/message")
                .or_else(|| payload.get("message"))
                .and_then(Value::as_str)
                .filter(|message| !message.trim().is_empty())
                .map(str::to_owned)
                .unwrap_or_else(|| format!("model endpoint returned HTTP {status}"));
            ModelError::Inference(message)
        }
        ureq::Error::Transport(error) => {
            ModelError::Inference(format!("model transport failed: {error}"))
        }
    }
}

pub fn extract_output_text(payload: &Value) -> Option<String> {
    if let Some(text) = payload.get("output_text").and_then(Value::as_str)
        && !text.is_empty()
    {
        return Some(text.to_string());
    }
    let output = payload.get("output").and_then(Value::as_array)?;
    let text = output
        .iter()
        .filter_map(|item| item.get("content").and_then(Value::as_array))
        .flatten()
        .filter_map(|content| content.get("text").and_then(Value::as_str))
        .collect::<Vec<_>>()
        .join("");
    (!text.is_empty()).then_some(text)
}

pub fn extract_usage(payload: &Value) -> Option<ModelUsage> {
    let usage = payload
        .get("usage")
        .or_else(|| payload.pointer("/response/usage"))?;
    let input_tokens = usage_value(usage, &["input_tokens", "prompt_tokens", "inputTokens"]);
    let output_tokens = usage_value(
        usage,
        &["output_tokens", "completion_tokens", "outputTokens"],
    );
    let cached_input_tokens = usage_value(usage, &["cached_input_tokens", "cachedInputTokens"])
        .max(
            usage
                .pointer("/input_tokens_details/cached_tokens")
                .and_then(Value::as_u64)
                .unwrap_or(0),
        );
    let reasoning_output_tokens =
        usage_value(usage, &["reasoning_output_tokens", "reasoningOutputTokens"]).max(
            usage
                .pointer("/output_tokens_details/reasoning_tokens")
                .and_then(Value::as_u64)
                .unwrap_or(0),
        );
    let total_tokens = usage_value(usage, &["total_tokens", "totalTokens"])
        .max(input_tokens.saturating_add(output_tokens));
    (total_tokens > 0).then_some(ModelUsage {
        total_tokens,
        input_tokens,
        cached_input_tokens,
        output_tokens,
        reasoning_output_tokens,
    })
}

fn usage_value(usage: &Value, keys: &[&str]) -> u64 {
    keys.iter()
        .find_map(|key| usage.get(*key).and_then(Value::as_u64))
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_text_and_usage_from_responses_payload() {
        let payload = json!({
            "output": [{"content": [{"type":"output_text", "text":"hello"}]}],
            "usage": {"input_tokens": 10, "output_tokens": 4, "total_tokens": 14}
        });
        assert_eq!(extract_output_text(&payload).as_deref(), Some("hello"));
        assert_eq!(
            extract_usage(&payload),
            Some(ModelUsage {
                total_tokens: 14,
                input_tokens: 10,
                cached_input_tokens: 0,
                output_tokens: 4,
                reasoning_output_tokens: 0,
            })
        );
    }

    #[test]
    fn remote_config_requires_https_and_rejects_header_injection() {
        let mut config = ResponsesModelConfig {
            base_url: "http://example.test/v1".into(),
            default_model: "model".into(),
            bearer_token: None,
            provider_mode: ModelProviderMode::FirstPartyDacheng,
            wire_api: ResponsesWireApi::Responses,
        };
        assert!(config.validate().is_err());
        config.base_url = "https://example.test/v1".into();
        config.bearer_token = Some("token\nheader".into());
        assert!(config.validate().is_err());
    }

    #[test]
    fn normalizes_chat_completion_text_tools_and_usage() {
        let normalized = normalize_chat_payload(json!({
            "id":"chat-1",
            "choices":[{"message":{"role":"assistant","content":"善哉","tool_calls":[{"id":"call-1","type":"function","function":{"name":"search","arguments":"{\"q\":\"法\"}"}}]}}],
            "usage":{"prompt_tokens":12,"completion_tokens":4,"total_tokens":16}
        }));
        assert_eq!(extract_output_text(&normalized).as_deref(), Some("善哉"));
        assert_eq!(normalized["output"][1]["name"], "search");
        assert_eq!(extract_usage(&normalized).unwrap().total_tokens, 16);
    }

    #[test]
    fn groups_adjacent_tool_calls_into_one_assistant_message() {
        let messages = chat_messages(&json!([
            {"role":"user", "content":"search"},
            {"type":"function_call", "call_id":"call-1", "name":"first", "arguments":"{}"},
            {"type":"function_call", "call_id":"call-2", "name":"second", "arguments":"{}"},
            {"type":"function_call_output", "call_id":"call-1", "output":"one"},
            {"type":"function_call_output", "call_id":"call-2", "output":"two"}
        ]));
        assert_eq!(messages.len(), 4);
        assert_eq!(messages[1]["tool_calls"].as_array().unwrap().len(), 2);
    }

    #[test]
    fn normalizes_anthropic_text_tools_and_cache_usage() {
        let normalized = normalize_anthropic_payload(json!({
            "id":"msg-1",
            "content":[
                {"type":"text","text":"善哉"},
                {"type":"tool_use","id":"tool-1","name":"search","input":{"q":"法"}}
            ],
            "usage":{"input_tokens":10,"cache_creation_input_tokens":2,"cache_read_input_tokens":4,"output_tokens":3}
        }));
        assert_eq!(extract_output_text(&normalized).as_deref(), Some("善哉"));
        assert_eq!(normalized["output"][1]["name"], "search");
        assert_eq!(extract_usage(&normalized).unwrap().input_tokens, 12);
        assert_eq!(extract_usage(&normalized).unwrap().cached_input_tokens, 4);
        assert_eq!(extract_usage(&normalized).unwrap().total_tokens, 19);
    }

    #[test]
    fn groups_anthropic_text_with_tool_use_and_serializes_tool_results() {
        let messages = anthropic_messages(&json!([
            {"role":"user", "content":"search"},
            {"role":"assistant", "content":"I will search."},
            {"type":"function_call", "call_id":"tool-1", "name":"search", "arguments":"{\"q\":\"法\"}"},
            {"type":"function_call_output", "call_id":"tool-1", "output":{"ok":true}}
        ]));
        assert_eq!(messages.len(), 3);
        assert_eq!(messages[1]["content"].as_array().unwrap().len(), 2);
        assert_eq!(messages[2]["content"][0]["content"], "{\"ok\":true}");
    }
}
