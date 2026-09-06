use serde_json::{Value, json};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum MarketplaceRouteError {
    InvalidInput,
    CommandMismatch,
    CommandNotFound,
    InvalidArguments,
    InvalidProjection,
}

fn strings(value: &Value, key: &str) -> Vec<String> {
    value
        .get(key)
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(Value::as_str)
                .map(str::trim)
                .filter(|item| !item.is_empty())
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default()
}

fn command_matches(command: &Value, requested: &str) -> bool {
    command
        .get("name")
        .and_then(Value::as_str)
        .is_some_and(|name| name.eq_ignore_ascii_case(requested))
        || strings(command, "aliases")
            .iter()
            .any(|alias| alias.eq_ignore_ascii_case(requested))
}

fn natural_score(command: &Value, normalized: &str) -> usize {
    let mut phrases = Vec::new();
    if let Some(name) = command.get("name").and_then(Value::as_str) {
        phrases.push(name.to_string());
    }
    if let Some(description) = command.get("description").and_then(Value::as_str) {
        phrases.push(description.to_string());
    }
    phrases.extend(strings(command, "aliases"));
    phrases.extend(strings(command, "naturalLanguageHints"));
    phrases
        .into_iter()
        .filter(|phrase| {
            let phrase = phrase.trim().to_lowercase();
            !phrase.is_empty() && normalized.contains(&phrase)
        })
        .count()
}

fn dispatch(
    plugin_id: &str,
    projection: &Value,
    command: &Value,
    arguments: Value,
) -> Result<Value, MarketplaceRouteError> {
    let surfaces = projection
        .get("surfaces")
        .and_then(Value::as_array)
        .ok_or(MarketplaceRouteError::InvalidProjection)?;
    let surface_id = command
        .get("surfaceId")
        .and_then(Value::as_str)
        .ok_or(MarketplaceRouteError::InvalidProjection)?;
    let surface = surfaces
        .iter()
        .find(|surface| surface.get("id").and_then(Value::as_str) == Some(surface_id))
        .cloned()
        .ok_or(MarketplaceRouteError::InvalidProjection)?;
    let tool = command
        .get("tool")
        .and_then(Value::as_str)
        .filter(|tool| !tool.trim().is_empty())
        .ok_or(MarketplaceRouteError::InvalidProjection)?;
    let command_name = command
        .get("name")
        .and_then(Value::as_str)
        .filter(|name| !name.trim().is_empty())
        .ok_or(MarketplaceRouteError::InvalidProjection)?;
    let approval = command
        .get("approval")
        .and_then(Value::as_str)
        .unwrap_or("required");
    let mut command_projection = command.clone();
    command_projection
        .as_object_mut()
        .ok_or(MarketplaceRouteError::InvalidProjection)?
        .insert(
            "slash".to_string(),
            Value::String(format!("/{plugin_id}:{command_name}")),
        );
    let mut execution = serde_json::Map::new();
    execution.insert(
        "kind".to_string(),
        surface
            .get("kind")
            .cloned()
            .ok_or(MarketplaceRouteError::InvalidProjection)?,
    );
    execution.insert("tool".to_string(), Value::String(tool.to_string()));
    for (source, target) in [
        ("url", "endpoint"),
        ("command", "command"),
        ("server", "server"),
    ] {
        if let Some(value) = surface.get(source) {
            execution.insert(target.to_string(), value.clone());
        }
    }
    Ok(json!({
        "protocol": "fabushi.miniapp.bot.v2",
        "miniAppId": plugin_id,
        "bot": projection.get("bot").cloned().unwrap_or(Value::Null),
        "command": command_projection,
        "arguments": arguments,
        "surface": surface,
        "approval": approval,
        "requiresApproval": approval != "none",
        "execution": Value::Object(execution),
    }))
}

pub(crate) fn route_marketplace_input(
    plugin_id: &str,
    projection: &Value,
    input: &str,
) -> Result<Value, MarketplaceRouteError> {
    let input = input.trim();
    if input.is_empty() || input.len() > 10_000 {
        return Err(MarketplaceRouteError::InvalidInput);
    }
    let commands = projection
        .get("commands")
        .and_then(Value::as_array)
        .ok_or(MarketplaceRouteError::InvalidProjection)?;
    if let Some(without_slash) = input.strip_prefix('/') {
        let split = without_slash.find(char::is_whitespace);
        let (head, arguments_text) = split
            .map(|index| (&without_slash[..index], without_slash[index..].trim()))
            .unwrap_or((without_slash, ""));
        let (target, requested) = head
            .split_once(':')
            .ok_or(MarketplaceRouteError::CommandNotFound)?;
        if !target.eq_ignore_ascii_case(plugin_id) {
            return Err(MarketplaceRouteError::CommandMismatch);
        }
        let command = commands
            .iter()
            .find(|command| command_matches(command, requested))
            .ok_or(MarketplaceRouteError::CommandNotFound)?;
        let arguments = if arguments_text.is_empty() {
            json!({})
        } else {
            let value = serde_json::from_str::<Value>(arguments_text)
                .map_err(|_| MarketplaceRouteError::InvalidArguments)?;
            if !value.is_object() {
                return Err(MarketplaceRouteError::InvalidArguments);
            }
            value
        };
        return dispatch(plugin_id, projection, command, arguments);
    }

    let normalized = input.to_lowercase();
    let mut best: Option<(&Value, usize)> = None;
    for command in commands {
        let score = natural_score(command, &normalized);
        if score > best.map(|(_, best_score)| best_score).unwrap_or(0) {
            best = Some((command, score));
        }
    }
    if let Some((command, score)) = best.filter(|(_, score)| *score > 0) {
        return dispatch(plugin_id, projection, command, json!({"input": input}));
    }
    let surface = projection
        .get("surfaces")
        .and_then(Value::as_array)
        .and_then(|surfaces| surfaces.first())
        .cloned()
        .unwrap_or(Value::Null);
    Ok(json!({
        "kind": "natural-language",
        "miniAppId": plugin_id,
        "bot": projection.get("bot").cloned().unwrap_or(Value::Null),
        "input": input,
        "suggestedCommand": Value::Null,
        "surface": surface,
        "requiresMahayanaPlanning": true,
    }))
}
