//! Long-lived product composition for Fabushi's existing AppHost and the
//! Rust-native Mahayana Harness.
//!
//! Presentation shells keep one `UnifiedAppHost` handle for their lifetime.
//! Existing FeatureHost/Codex/plugin requests are delegated unchanged while
//! `harness.*` methods share one in-process Harness graph and a replayable JSONL
//! journal. The journal preserves stable external identifiers across restarts by
//! translating them to the freshly replayed runtime identifiers.

use mahayana_app_host::{
    AppHost, AppHostError, AppHostFeatureMode, HostRequest, HostResponse, default_app_data_dir,
};
use mahayana_core::BuildProfile;
use mahayana_harness_protocol::HarnessApi;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use std::collections::BTreeMap;
use std::fs::{File, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

const JOURNAL_VERSION: u32 = 1;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct JournalRecord {
    version: u32,
    operation: String,
    payload: Value,
    result: Value,
}

struct HarnessJournal {
    path: PathBuf,
    stable_to_runtime: BTreeMap<String, String>,
}

impl HarnessJournal {
    fn open(root: &Path, api: &HarnessApi) -> Result<Self, AppHostError> {
        std::fs::create_dir_all(root)
            .map_err(|error| AppHostError::Operation(error.to_string()))?;
        let path = root.join("operations.jsonl");
        let mut journal = Self {
            path,
            stable_to_runtime: BTreeMap::new(),
        };
        journal.replay(api)?;
        Ok(journal)
    }

    fn replay(&mut self, api: &HarnessApi) -> Result<(), AppHostError> {
        if !self.path.is_file() {
            return Ok(());
        }
        let file =
            File::open(&self.path).map_err(|error| AppHostError::Operation(error.to_string()))?;
        for (index, line) in BufReader::new(file).lines().enumerate() {
            let line = line.map_err(|error| AppHostError::Operation(error.to_string()))?;
            if line.trim().is_empty() {
                continue;
            }
            let record: JournalRecord = serde_json::from_str(&line).map_err(|error| {
                AppHostError::Operation(format!(
                    "invalid harness journal record {}: {error}",
                    index + 1
                ))
            })?;
            if record.version != JOURNAL_VERSION {
                return Err(AppHostError::Operation(format!(
                    "unsupported harness journal version {}",
                    record.version
                )));
            }
            let runtime_payload = rewrite_ids(
                &record.payload,
                &self.stable_to_runtime,
                RewriteDirection::StableToRuntime,
                None,
            );
            let runtime_result =
                api.execute(&record.operation, runtime_payload)
                    .map_err(|error| {
                        AppHostError::Operation(format!(
                            "failed to replay harness operation {}: {error}",
                            record.operation
                        ))
                    })?;
            collect_id_aliases(
                &record.result,
                &runtime_result,
                None,
                &mut self.stable_to_runtime,
            );
        }
        Ok(())
    }

    fn execute(
        &mut self,
        api: &HarnessApi,
        operation: &str,
        payload: Value,
    ) -> Result<Value, AppHostError> {
        let runtime_payload = rewrite_ids(
            &payload,
            &self.stable_to_runtime,
            RewriteDirection::StableToRuntime,
            None,
        );
        let runtime_result = api.execute(operation, runtime_payload).map_err(|error| {
            AppHostError::Operation(format!("harness operation failed: {error}"))
        })?;
        let stable_result = rewrite_ids(
            &runtime_result,
            &self.stable_to_runtime,
            RewriteDirection::RuntimeToStable,
            None,
        );
        if is_journaled_operation(operation) {
            self.append(&JournalRecord {
                version: JOURNAL_VERSION,
                operation: operation.to_string(),
                payload,
                result: stable_result.clone(),
            })?;
        }
        Ok(stable_result)
    }

    fn append(&self, record: &JournalRecord) -> Result<(), AppHostError> {
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)
            .map_err(|error| AppHostError::Operation(error.to_string()))?;
        serde_json::to_writer(&mut file, record)
            .map_err(|error| AppHostError::Operation(error.to_string()))?;
        file.write_all(b"\n")
            .and_then(|_| file.flush())
            .map_err(|error| AppHostError::Operation(error.to_string()))
    }
}

#[derive(Debug, Clone, Copy)]
enum RewriteDirection {
    StableToRuntime,
    RuntimeToStable,
}

pub struct UnifiedAppHost {
    app: AppHost,
    harness: HarnessApi,
    journal: Mutex<HarnessJournal>,
}

impl UnifiedAppHost {
    pub fn new(app_data_dir: impl Into<PathBuf>) -> Result<Self, AppHostError> {
        let app_data_dir = app_data_dir.into();
        let app = AppHost::new(app_data_dir.clone())?;
        Self::from_app(app_data_dir, app)
    }

    pub fn new_with_feature_mode(
        app_data_dir: impl Into<PathBuf>,
        feature_mode: AppHostFeatureMode,
    ) -> Result<Self, AppHostError> {
        let app_data_dir = app_data_dir.into();
        let app = AppHost::new_with_feature_mode(app_data_dir.clone(), feature_mode)?;
        Self::from_app(app_data_dir, app)
    }

    pub fn new_with_feature_mode_and_storage_passphrase(
        app_data_dir: impl Into<PathBuf>,
        feature_mode: AppHostFeatureMode,
        storage_passphrase: String,
    ) -> Result<Self, AppHostError> {
        let app_data_dir = app_data_dir.into();
        let app = AppHost::new_with_feature_mode_and_storage_passphrase(
            app_data_dir.clone(),
            feature_mode,
            Some(storage_passphrase),
        )?;
        Self::from_app(app_data_dir, app)
    }

    fn from_app(app_data_dir: PathBuf, app: AppHost) -> Result<Self, AppHostError> {
        let harness = HarnessApi::new(harness_build_profile());
        let journal = HarnessJournal::open(&app_data_dir.join("harness"), &harness)?;
        Ok(Self {
            app,
            harness,
            journal: Mutex::new(journal),
        })
    }

    pub fn dispatch(&self, request: HostRequest) -> HostResponse {
        if !request.method.starts_with("harness.") {
            return self.app.dispatch(request);
        }
        let id = request.id.clone();
        match self.handle_harness(&request.method, request.params) {
            Ok(result) => HostResponse {
                id,
                ok: true,
                result: Some(result),
                error: None,
            },
            Err(error) => HostResponse {
                id,
                ok: false,
                result: None,
                error: Some(error.to_string()),
            },
        }
    }

    fn handle_harness(&self, method: &str, params: Value) -> Result<Value, AppHostError> {
        match method {
            "harness.execute" => {
                let operation = required_string(&params, "operation")?;
                let payload = params.get("payload").cloned().unwrap_or(Value::Null);
                self.execute_harness(&operation, payload)
            }
            "harness.snapshot" => self.execute_harness("runtime.snapshot", Value::Null),
            "harness.services" => self.execute_harness("services.snapshot", Value::Null),
            "harness.chat.send" => self.harness_chat_send(params),
            "harness.feature.execute" => self.harness_feature_execute(params),
            "harness.feature.receive" => self.harness_feature_receive(params),
            "harness.approval.resolve" => self.harness_approval_resolve(params),
            "harness.interrupt" => self.harness_interrupt(params),
            other => Err(AppHostError::InvalidRequest(format!(
                "unknown harness method {other}"
            ))),
        }
    }

    fn execute_harness(&self, operation: &str, payload: Value) -> Result<Value, AppHostError> {
        match operation {
            "schedule.create" => self.harness_schedule_create(payload),
            "schedule.setEnabled" => self.harness_schedule_set_enabled(payload),
            "schedule.delete" => self.harness_schedule_delete(payload),
            "schedule.run" => self.harness_schedule_run(payload),
            _ => self.execute_harness_state(operation, payload),
        }
    }

    fn execute_harness_state(
        &self,
        operation: &str,
        payload: Value,
    ) -> Result<Value, AppHostError> {
        self.journal
            .lock()
            .map_err(|_| AppHostError::Operation("harness journal lock poisoned".into()))?
            .execute(&self.harness, operation, payload)
    }

    fn harness_schedule_create(&self, payload: Value) -> Result<Value, AppHostError> {
        let created = self.execute_harness_state("schedule.create", payload)?;
        let id = required_string(&created, "id")?;
        let session_id = required_string(&created, "sessionId")?;
        let instruction = required_string(&created, "instruction")?;
        let schedule = required_string(&created, "schedule")?;
        let command = json!({"command": {
            "type": "automation.upsert",
            "requestId": format!("harness-schedule-upsert:{}:{}", id, now_ms()),
            "id": id.clone(),
            "name": format!("Harness schedule · {session_id}"),
            "prompt": instruction,
            "schedule": schedule.clone(),
            "trigger": {"kind": "schedule", "schedule": schedule},
            "enabled": true
        }});
        if let Err(error) = self.delegate("feature.execute", command) {
            let _ = self.execute_harness_state("schedule.delete", json!({"scheduleId": id}));
            return Err(error);
        }
        Ok(created)
    }

    fn harness_schedule_set_enabled(&self, payload: Value) -> Result<Value, AppHostError> {
        let id = required_string(&payload, "scheduleId")?;
        let enabled = payload
            .get("enabled")
            .and_then(Value::as_bool)
            .ok_or_else(|| AppHostError::InvalidRequest("enabled is required".into()))?;
        self.delegate(
            "feature.execute",
            json!({"command": {
                "type": "automation.setEnabled",
                "requestId": format!("harness-schedule-enabled:{}:{}", id, now_ms()),
                "id": id.clone(),
                "enabled": enabled
            }}),
        )?;
        self.execute_harness_state("schedule.setEnabled", payload)
    }

    fn harness_schedule_delete(&self, payload: Value) -> Result<Value, AppHostError> {
        let id = required_string(&payload, "scheduleId")?;
        self.delegate(
            "feature.execute",
            json!({"command": {
                "type": "automation.delete",
                "requestId": format!("harness-schedule-delete:{}:{}", id, now_ms()),
                "id": id.clone()
            }}),
        )?;
        self.execute_harness_state("schedule.delete", payload)
    }

    fn harness_schedule_run(&self, payload: Value) -> Result<Value, AppHostError> {
        let id = required_string(&payload, "scheduleId")?;
        let schedule =
            self.execute_harness_state("schedule.get", json!({"scheduleId": id.clone()}))?;
        let session_id = required_string(&schedule, "sessionId")?;
        let accepted = self.delegate(
            "feature.execute",
            json!({"command": {
                "type": "automation.run",
                "requestId": format!("harness-schedule-run:{}:{}", id, now_ms()),
                "id": id.clone()
            }}),
        )?;
        self.execute_harness_state(
            "session.appendEvent",
            json!({
                "sessionId": session_id,
                "kind": "schedule/run",
                "eventPayload": {"scheduleId": id, "accepted": accepted.clone()}
            }),
        )?;
        Ok(json!({"schedule": schedule, "accepted": accepted}))
    }

    fn harness_chat_send(&self, params: Value) -> Result<Value, AppHostError> {
        let session_id = required_string(&params, "sessionId")?;
        let text = required_string(&params, "text")?;
        let request_id = params
            .get("requestId")
            .and_then(Value::as_str)
            .filter(|value| !value.trim().is_empty())
            .map(str::to_string)
            .unwrap_or_else(|| format!("harness:{}", now_ms()));

        self.execute_harness(
            "session.appendEvent",
            json!({
                "sessionId": session_id,
                "kind": "user/message",
                "eventPayload": {
                    "text": text,
                    "requestId": request_id,
                }
            }),
        )?;

        let mut command = Map::new();
        command.insert("type".into(), Value::String("chat.send".into()));
        command.insert("requestId".into(), Value::String(request_id.clone()));
        command.insert("text".into(), Value::String(text));
        for key in [
            "agentId",
            "conversationId",
            "mode",
            "modeStatement",
            "model",
            "attachments",
        ] {
            if let Some(value) = params.get(key) {
                command.insert(key.into(), value.clone());
            }
        }

        let accepted = self.delegate(
            "feature.execute",
            json!({"command": Value::Object(command)}),
        )?;
        self.execute_harness(
            "session.appendEvent",
            json!({
                "sessionId": session_id,
                "kind": "feature/accepted",
                "eventPayload": {
                    "requestId": request_id,
                    "accepted": accepted,
                }
            }),
        )?;
        Ok(json!({
            "sessionId": session_id,
            "requestId": request_id,
            "accepted": accepted,
        }))
    }

    fn harness_feature_execute(&self, params: Value) -> Result<Value, AppHostError> {
        let session_id = required_string(&params, "sessionId")?;
        let command = params
            .get("command")
            .cloned()
            .ok_or_else(|| AppHostError::InvalidRequest("command is required".into()))?;
        self.execute_harness(
            "session.appendEvent",
            json!({
                "sessionId": session_id,
                "kind": "feature/command",
                "eventPayload": {"command": command}
            }),
        )?;
        let accepted = self.delegate("feature.execute", json!({"command": command}))?;
        self.execute_harness(
            "session.appendEvent",
            json!({
                "sessionId": session_id,
                "kind": "feature/accepted",
                "eventPayload": {"accepted": accepted}
            }),
        )?;
        Ok(accepted)
    }

    fn harness_feature_receive(&self, params: Value) -> Result<Value, AppHostError> {
        let session_id = required_string(&params, "sessionId")?;
        let event = self.delegate("feature.receive", Value::Null)?;
        if !event.is_null() {
            self.execute_harness(
                "session.appendEvent",
                json!({
                    "sessionId": session_id,
                    "kind": "feature/event",
                    "eventPayload": event
                }),
            )?;
        }
        Ok(event)
    }

    fn harness_approval_resolve(&self, params: Value) -> Result<Value, AppHostError> {
        let session_id = required_string(&params, "sessionId")?;
        let resolution = params
            .get("resolution")
            .cloned()
            .ok_or_else(|| AppHostError::InvalidRequest("resolution is required".into()))?;
        let result = self.delegate(
            "feature.approval.resolve",
            json!({"resolution": resolution}),
        )?;
        self.execute_harness(
            "session.appendEvent",
            json!({
                "sessionId": session_id,
                "kind": "feature/approval-resolved",
                "eventPayload": {"resolution": resolution}
            }),
        )?;
        Ok(result)
    }

    fn harness_interrupt(&self, params: Value) -> Result<Value, AppHostError> {
        let session_id = required_string(&params, "sessionId")?;
        let operation_id = required_string(&params, "operationId")?;
        let result = self.delegate("feature.interrupt", json!({"operationId": operation_id}))?;
        self.execute_harness(
            "session.appendEvent",
            json!({
                "sessionId": session_id,
                "kind": "feature/interrupted",
                "eventPayload": {"operationId": operation_id}
            }),
        )?;
        Ok(result)
    }

    fn delegate(&self, method: &str, params: Value) -> Result<Value, AppHostError> {
        let response = self.app.dispatch(HostRequest {
            id: None,
            method: method.to_string(),
            params,
        });
        if response.ok {
            Ok(response.result.unwrap_or(Value::Null))
        } else {
            Err(AppHostError::Operation(response.error.unwrap_or_else(
                || format!("delegated host method {method} failed"),
            )))
        }
    }
}

pub fn dispatch_json(host: &UnifiedAppHost, input: &str) -> String {
    let response = match serde_json::from_str::<HostRequest>(input) {
        Ok(request) => host.dispatch(request),
        Err(error) => HostResponse {
            id: None,
            ok: false,
            result: None,
            error: Some(format!("invalid JSON request: {error}")),
        },
    };
    serde_json::to_string(&response).unwrap_or_else(|error| {
        format!("{{\"ok\":false,\"error\":\"serialization failed: {error}\"}}")
    })
}

pub fn default_unified_app_data_dir() -> PathBuf {
    default_app_data_dir()
}

fn is_journaled_operation(operation: &str) -> bool {
    matches!(
        operation,
        "service.register"
            | "service.unregister"
            | "profile.register"
            | "profile.activate"
            | "plugin.mount"
            | "plugin.unmount"
            | "session.create"
            | "session.fork"
            | "session.appendEvent"
            | "agent.spawn"
            | "goal.create"
            | "goal.update"
            | "job.create"
            | "job.update"
            | "prompt.register"
            | "context.inject"
            | "workspace.register"
            | "settings.set"
            | "todo.add"
            | "todo.update"
            | "plan.set"
            | "plan.exit"
            | "schedule.create"
            | "schedule.setEnabled"
            | "schedule.delete"
            | "feedback.record"
            | "identity.bindAccount"
            | "team.create"
            | "team.addTask"
            | "team.sendMessage"
            | "skill.register"
            | "command.register"
            | "interaction.permissionPreset.register"
            | "interaction.permissionPreset.activate"
            | "interaction.question.create"
            | "interaction.question.answer"
            | "agentPlan.enter"
            | "agentPlan.exit"
            | "bundle.register"
            | "preset.register"
            | "extension.register"
            | "extension.setEnabled"
            | "hook.register"
    )
}

fn collect_id_aliases(
    stable: &Value,
    runtime: &Value,
    key: Option<&str>,
    aliases: &mut BTreeMap<String, String>,
) {
    match (stable, runtime) {
        (Value::String(stable), Value::String(runtime)) if key.is_some_and(is_id_key) => {
            aliases.insert(stable.clone(), runtime.clone());
        }
        (Value::Object(stable), Value::Object(runtime)) => {
            for (name, stable_value) in stable {
                if let Some(runtime_value) = runtime.get(name) {
                    collect_id_aliases(stable_value, runtime_value, Some(name), aliases);
                }
            }
        }
        (Value::Array(stable), Value::Array(runtime)) => {
            for (stable_value, runtime_value) in stable.iter().zip(runtime) {
                collect_id_aliases(stable_value, runtime_value, key, aliases);
            }
        }
        _ => {}
    }
}

fn rewrite_ids(
    value: &Value,
    aliases: &BTreeMap<String, String>,
    direction: RewriteDirection,
    key: Option<&str>,
) -> Value {
    match value {
        Value::String(value) if key.is_some_and(is_id_key) => match direction {
            RewriteDirection::StableToRuntime => aliases
                .get(value)
                .cloned()
                .map(Value::String)
                .unwrap_or_else(|| Value::String(value.clone())),
            RewriteDirection::RuntimeToStable => aliases
                .iter()
                .find_map(|(stable, runtime)| (runtime == value).then(|| stable.clone()))
                .map(Value::String)
                .unwrap_or_else(|| Value::String(value.clone())),
        },
        Value::Object(values) => Value::Object(
            values
                .iter()
                .map(|(name, value)| {
                    (
                        name.clone(),
                        rewrite_ids(value, aliases, direction, Some(name)),
                    )
                })
                .collect(),
        ),
        Value::Array(values) => Value::Array(
            values
                .iter()
                .map(|value| rewrite_ids(value, aliases, direction, key))
                .collect(),
        ),
        _ => value.clone(),
    }
}

fn is_id_key(key: &str) -> bool {
    key == "id"
        || key.ends_with("Id")
        || key.ends_with("Ids")
        || key.ends_with("_id")
        || key.ends_with("_ids")
}

fn required_string(params: &Value, key: &str) -> Result<String, AppHostError> {
    params
        .get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .ok_or_else(|| AppHostError::InvalidRequest(format!("{key} is required")))
}

fn harness_build_profile() -> BuildProfile {
    if cfg!(any(target_os = "ios", target_os = "android")) {
        BuildProfile::MobileEmbedded
    } else {
        BuildProfile::DesktopFull
    }
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis().min(i64::MAX as u128) as i64)
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn unique_test_dir() -> PathBuf {
        std::env::temp_dir().join(format!(
            "fabushi-unified-harness-{}-{}",
            std::process::id(),
            now_ms()
        ))
    }

    fn call(host: &UnifiedAppHost, method: &str, params: Value) -> Value {
        let response = host.dispatch(HostRequest {
            id: Some(json!(1)),
            method: method.to_string(),
            params,
        });
        assert!(response.ok, "{:?}", response.error);
        response.result.unwrap_or(Value::Null)
    }

    #[test]
    fn harness_state_survives_host_restart_with_stable_ids() {
        let root = unique_test_dir();
        let stable_session_id = {
            let host = UnifiedAppHost::new_with_feature_mode(&root, AppHostFeatureMode::Test)
                .expect("create unified test host");
            let session = call(
                &host,
                "harness.execute",
                json!({
                    "operation": "session.create",
                    "payload": {"title": "persistent"}
                }),
            );
            let session_id = session["id"].as_str().unwrap().to_string();
            call(
                &host,
                "harness.execute",
                json!({
                    "operation": "session.appendEvent",
                    "payload": {
                        "sessionId": session_id,
                        "kind": "user/message",
                        "eventPayload": {"text": "survives restart"}
                    }
                }),
            );
            session_id
        };

        let host = UnifiedAppHost::new_with_feature_mode(&root, AppHostFeatureMode::Test)
            .expect("reopen unified test host");
        let transcript = call(
            &host,
            "harness.execute",
            json!({
                "operation": "session.transcript",
                "payload": {"sessionId": stable_session_id}
            }),
        );
        assert_eq!(transcript["transcript"], "user: survives restart");

        let snapshot = call(&host, "harness.snapshot", Value::Null);
        assert_eq!(snapshot["sessions"][0]["id"], stable_session_id);
        let _ = std::fs::remove_dir_all(root);
    }
}
