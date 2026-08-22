use crate::{ModelError, ModelEvent, ModelRequest, ModelRuntime, ModelUsage, SharedModelEventSink};
use async_trait::async_trait;
use mahayana_core::ModelProviderMode;
use serde_json::{Value, json};

#[derive(Debug, Clone)]
pub struct ResponsesModelConfig {
    pub base_url: String,
    pub default_model: String,
    pub bearer_token: Option<String>,
    pub provider_mode: ModelProviderMode,
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
}

impl ResponsesModelRuntime {
    pub fn new(config: ResponsesModelConfig) -> Result<Self, ModelError> {
        config.validate()?;
        Ok(Self { config })
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

        let config = self.config.clone();
        let payload = tokio::task::spawn_blocking(move || request_response(&config, request))
            .await
            .map_err(|error| ModelError::Inference(format!("model task failed: {error}")))??;

        if let Some(text) = extract_output_text(&payload) {
            events.emit(ModelEvent::OutputTextDelta(text))?;
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
) -> Result<Value, ModelError> {
    let endpoint = if config.base_url.ends_with("/responses") {
        config.base_url.clone()
    } else {
        format!("{}/responses", config.base_url.trim_end_matches('/'))
    };

    let mut body = json!({
        "model": request.model,
        "input": request.input,
        "stream": false,
    });
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

    let mut http = ureq::post(&endpoint).set("Accept", "application/json");
    if let Some(token) = config.bearer_token.as_deref() {
        http = http.set("Authorization", &format!("Bearer {token}"));
    }
    let response = http.send_json(body).map_err(redacted_http_error)?;
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
    Ok(payload)
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
    if let Some(text) = payload
        .get("output_text")
        .and_then(Value::as_str)
        .filter(|text| !text.is_empty())
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
        };
        assert!(config.validate().is_err());
        config.base_url = "https://example.test/v1".into();
        config.bearer_token = Some("token\nheader".into());
        assert!(config.validate().is_err());
    }
}
