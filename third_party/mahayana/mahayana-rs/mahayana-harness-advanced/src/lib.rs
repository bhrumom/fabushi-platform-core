//! Advanced product semantics for the Rust-native Mahayana Harness.
//!
//! This crate keeps product-level policy and orchestration state separate from
//! the minimal core event/tool loop. It covers the capability families that a
//! real product shell needs on top of the core: interaction policy, per-agent
//! plan mode, bundles/presets/extensions/hooks, bounded session queries,
//! deterministic title fallback, and telemetry buffering.

use mahayana_harness::{HarnessError, HarnessEvent, HarnessResult, MahayanaHarness};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{SystemTime, UNIX_EPOCH};
use uuid::Uuid;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PermissionPreset {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub allowed_tools: Vec<String>,
    #[serde(default)]
    pub denied_tools: Vec<String>,
    #[serde(default)]
    pub require_approval_for_writes: bool,
    #[serde(default)]
    pub metadata: Value,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct QuestionOption {
    pub id: String,
    pub label: String,
    #[serde(default)]
    pub description: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UserQuestion {
    pub id: String,
    pub session_id: String,
    #[serde(default)]
    pub agent_id: Option<String>,
    pub prompt: String,
    #[serde(default)]
    pub options: Vec<QuestionOption>,
    #[serde(default)]
    pub allow_freeform: bool,
    pub status: String,
    #[serde(default)]
    pub answer: Option<Value>,
    pub created_at_ms: i64,
    #[serde(default)]
    pub answered_at_ms: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentPlanState {
    pub agent_id: String,
    pub session_id: String,
    pub mode: String,
    #[serde(default)]
    pub steps: Vec<String>,
    pub review_required: bool,
    pub entered_at_ms: i64,
    pub updated_at_ms: i64,
    #[serde(default)]
    pub reviewed_by: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BundleDefinition {
    pub id: String,
    #[serde(default)]
    pub services: Vec<String>,
    #[serde(default)]
    pub plugins: Vec<String>,
    #[serde(default)]
    pub settings: Value,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PresetDefinition {
    pub id: String,
    #[serde(default)]
    pub bundles: Vec<String>,
    #[serde(default)]
    pub prompt_sections: Vec<String>,
    #[serde(default)]
    pub tool_allowlist: Vec<String>,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub metadata: Value,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ResolvedPreset {
    pub preset: PresetDefinition,
    pub bundles: Vec<BundleDefinition>,
    pub settings: Value,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExtensionDefinition {
    pub id: String,
    pub version: String,
    pub enabled: bool,
    #[serde(default)]
    pub services: Vec<String>,
    #[serde(default)]
    pub tools: Vec<String>,
    #[serde(default)]
    pub metadata: Value,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HookRegistration {
    pub id: String,
    pub event: String,
    pub handler: String,
    pub priority: i32,
    pub enabled: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HookEmission {
    pub id: String,
    pub event: String,
    pub payload: Value,
    pub handler_ids: Vec<String>,
    pub created_at_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TelemetryRecord {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub attributes: Value,
    pub timestamp_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TitleCandidate {
    pub session_id: String,
    pub title: String,
    pub source: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionEventPage {
    pub events: Vec<HarnessEvent>,
    #[serde(default)]
    pub next_after_sequence: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionLineage {
    pub session_id: String,
    pub ancestors: Vec<String>,
    pub descendants: Vec<String>,
}

#[derive(Debug, Default)]
struct AdvancedState {
    permission_presets: BTreeMap<String, PermissionPreset>,
    active_permission_preset: Option<String>,
    questions: BTreeMap<String, UserQuestion>,
    plans: BTreeMap<String, AgentPlanState>,
    bundles: BTreeMap<String, BundleDefinition>,
    presets: BTreeMap<String, PresetDefinition>,
    extensions: BTreeMap<String, ExtensionDefinition>,
    hooks: BTreeMap<String, HookRegistration>,
    hook_emissions: VecDeque<HookEmission>,
    telemetry: VecDeque<TelemetryRecord>,
}

#[derive(Clone)]
pub struct AdvancedHarnessServices {
    harness: MahayanaHarness,
    state: Arc<Mutex<AdvancedState>>,
}

impl AdvancedHarnessServices {
    pub fn new(harness: MahayanaHarness) -> Self {
        Self {
            harness,
            state: Arc::new(Mutex::new(AdvancedState::default())),
        }
    }

    pub fn harness(&self) -> &MahayanaHarness {
        &self.harness
    }

    pub fn register_permission_preset(
        &self,
        preset: PermissionPreset,
    ) -> HarnessResult<PermissionPreset> {
        let id = required(&preset.id, "permission preset id")?;
        let mut state = self.state()?;
        state.permission_presets.insert(id, preset.clone());
        Ok(preset)
    }

    pub fn permission_presets(&self) -> HarnessResult<Vec<PermissionPreset>> {
        Ok(self.state()?.permission_presets.values().cloned().collect())
    }

    pub fn activate_permission_preset(&self, preset_id: &str) -> HarnessResult<PermissionPreset> {
        let mut state = self.state()?;
        let preset = state
            .permission_presets
            .get(preset_id)
            .cloned()
            .ok_or_else(|| {
                HarnessError::ServiceNotFound(format!("permission-preset:{preset_id}"))
            })?;
        state.active_permission_preset = Some(preset_id.to_string());
        Ok(preset)
    }

    pub fn active_permission_preset(&self) -> HarnessResult<Option<PermissionPreset>> {
        let state = self.state()?;
        Ok(state
            .active_permission_preset
            .as_deref()
            .and_then(|id| state.permission_presets.get(id))
            .cloned())
    }

    pub fn ask_question(
        &self,
        session_id: impl Into<String>,
        agent_id: Option<String>,
        prompt: impl Into<String>,
        options: Vec<QuestionOption>,
        allow_freeform: bool,
    ) -> HarnessResult<UserQuestion> {
        let session_id = required(&session_id.into(), "session id")?;
        self.ensure_session(&session_id)?;
        let question = UserQuestion {
            id: format!("question:{}", Uuid::new_v4()),
            session_id,
            agent_id,
            prompt: required(&prompt.into(), "question prompt")?,
            options,
            allow_freeform,
            status: "pending".into(),
            answer: None,
            created_at_ms: now_ms(),
            answered_at_ms: None,
        };
        self.state()?
            .questions
            .insert(question.id.clone(), question.clone());
        Ok(question)
    }

    pub fn answer_question(&self, question_id: &str, answer: Value) -> HarnessResult<UserQuestion> {
        let mut state = self.state()?;
        let question = state
            .questions
            .get_mut(question_id)
            .ok_or_else(|| HarnessError::ServiceNotFound(format!("question:{question_id}")))?;
        if question.status != "pending" {
            return Err(HarnessError::InvalidConfig(format!(
                "question {question_id} is already {}",
                question.status
            )));
        }
        validate_question_answer(question, &answer)?;
        question.status = "answered".into();
        question.answer = Some(answer);
        question.answered_at_ms = Some(now_ms());
        Ok(question.clone())
    }

    pub fn questions(&self, pending_only: bool) -> HarnessResult<Vec<UserQuestion>> {
        Ok(self
            .state()?
            .questions
            .values()
            .filter(|question| !pending_only || question.status == "pending")
            .cloned()
            .collect())
    }

    pub fn enter_plan(
        &self,
        agent_id: impl Into<String>,
        session_id: impl Into<String>,
        steps: Vec<String>,
        review_required: bool,
    ) -> HarnessResult<AgentPlanState> {
        let agent_id = required(&agent_id.into(), "agent id")?;
        let session_id = required(&session_id.into(), "session id")?;
        self.ensure_session(&session_id)?;
        let now = now_ms();
        let plan = AgentPlanState {
            agent_id: agent_id.clone(),
            session_id,
            mode: "plan".into(),
            steps,
            review_required,
            entered_at_ms: now,
            updated_at_ms: now,
            reviewed_by: None,
        };
        self.state()?.plans.insert(agent_id, plan.clone());
        Ok(plan)
    }

    pub fn exit_plan(
        &self,
        agent_id: &str,
        approved: bool,
        reviewer: Option<String>,
    ) -> HarnessResult<AgentPlanState> {
        let mut state = self.state()?;
        let plan = state
            .plans
            .get_mut(agent_id)
            .ok_or_else(|| HarnessError::ServiceNotFound(format!("agent-plan:{agent_id}")))?;
        if plan.review_required && !approved {
            return Err(HarnessError::ApprovalRequired(format!(
                "agent-plan:{agent_id}"
            )));
        }
        plan.mode = "execution".into();
        plan.reviewed_by = reviewer;
        plan.updated_at_ms = now_ms();
        Ok(plan.clone())
    }

    pub fn agent_plan(&self, agent_id: &str) -> HarnessResult<Option<AgentPlanState>> {
        Ok(self.state()?.plans.get(agent_id).cloned())
    }

    pub fn register_bundle(&self, bundle: BundleDefinition) -> HarnessResult<BundleDefinition> {
        let id = required(&bundle.id, "bundle id")?;
        self.state()?.bundles.insert(id, bundle.clone());
        Ok(bundle)
    }

    pub fn bundles(&self) -> HarnessResult<Vec<BundleDefinition>> {
        Ok(self.state()?.bundles.values().cloned().collect())
    }

    pub fn register_preset(&self, preset: PresetDefinition) -> HarnessResult<PresetDefinition> {
        let id = required(&preset.id, "preset id")?;
        self.state()?.presets.insert(id, preset.clone());
        Ok(preset)
    }

    pub fn presets(&self) -> HarnessResult<Vec<PresetDefinition>> {
        Ok(self.state()?.presets.values().cloned().collect())
    }

    pub fn resolve_preset(&self, preset_id: &str) -> HarnessResult<ResolvedPreset> {
        let state = self.state()?;
        let preset = state
            .presets
            .get(preset_id)
            .cloned()
            .ok_or_else(|| HarnessError::ServiceNotFound(format!("preset:{preset_id}")))?;
        let mut bundles = Vec::with_capacity(preset.bundles.len());
        let mut settings = Map::new();
        for bundle_id in &preset.bundles {
            let bundle = state
                .bundles
                .get(bundle_id)
                .cloned()
                .ok_or_else(|| HarnessError::ServiceNotFound(format!("bundle:{bundle_id}")))?;
            if let Value::Object(values) = &bundle.settings {
                for (key, value) in values {
                    settings.insert(key.clone(), value.clone());
                }
            }
            bundles.push(bundle);
        }
        Ok(ResolvedPreset {
            preset,
            bundles,
            settings: Value::Object(settings),
        })
    }

    pub fn register_extension(
        &self,
        extension: ExtensionDefinition,
    ) -> HarnessResult<ExtensionDefinition> {
        let id = required(&extension.id, "extension id")?;
        self.state()?.extensions.insert(id, extension.clone());
        Ok(extension)
    }

    pub fn set_extension_enabled(
        &self,
        extension_id: &str,
        enabled: bool,
    ) -> HarnessResult<ExtensionDefinition> {
        let mut state = self.state()?;
        let extension = state
            .extensions
            .get_mut(extension_id)
            .ok_or_else(|| HarnessError::ServiceNotFound(format!("extension:{extension_id}")))?;
        extension.enabled = enabled;
        Ok(extension.clone())
    }

    pub fn extensions(&self) -> HarnessResult<Vec<ExtensionDefinition>> {
        Ok(self.state()?.extensions.values().cloned().collect())
    }

    pub fn register_hook(&self, hook: HookRegistration) -> HarnessResult<HookRegistration> {
        let id = required(&hook.id, "hook id")?;
        required(&hook.event, "hook event")?;
        required(&hook.handler, "hook handler")?;
        self.state()?.hooks.insert(id, hook.clone());
        Ok(hook)
    }

    pub fn hooks(&self) -> HarnessResult<Vec<HookRegistration>> {
        Ok(self.state()?.hooks.values().cloned().collect())
    }

    pub fn emit_hook(
        &self,
        event: impl Into<String>,
        payload: Value,
    ) -> HarnessResult<HookEmission> {
        let event = required(&event.into(), "hook event")?;
        let mut state = self.state()?;
        let mut hooks = state
            .hooks
            .values()
            .filter(|hook| hook.enabled && hook.event == event)
            .cloned()
            .collect::<Vec<_>>();
        hooks.sort_by_key(|hook| (hook.priority, hook.id.clone()));
        let emission = HookEmission {
            id: format!("hook-emission:{}", Uuid::new_v4()),
            event,
            payload,
            handler_ids: hooks.into_iter().map(|hook| hook.id).collect(),
            created_at_ms: now_ms(),
        };
        state.hook_emissions.push_back(emission.clone());
        while state.hook_emissions.len() > 1024 {
            state.hook_emissions.pop_front();
        }
        Ok(emission)
    }

    pub fn session_events(
        &self,
        session_id: &str,
        after_sequence: Option<u64>,
        limit: usize,
    ) -> HarnessResult<SessionEventPage> {
        let snapshot = self.harness.snapshot()?;
        let session = snapshot
            .sessions
            .into_iter()
            .find(|session| session.id == session_id)
            .ok_or_else(|| HarnessError::SessionNotFound(session_id.into()))?;
        let limit = limit.clamp(1, 500);
        let after = after_sequence.unwrap_or(0);
        let mut matching = session
            .events
            .into_iter()
            .filter(|event| event.sequence > after)
            .collect::<Vec<_>>();
        matching.sort_by_key(|event| event.sequence);
        let has_more = matching.len() > limit;
        matching.truncate(limit);
        let next_after_sequence = has_more
            .then(|| matching.last().map(|event| event.sequence))
            .flatten();
        Ok(SessionEventPage {
            events: matching,
            next_after_sequence,
        })
    }

    pub fn session_lineage(&self, session_id: &str) -> HarnessResult<SessionLineage> {
        let snapshot = self.harness.snapshot()?;
        let sessions = snapshot
            .sessions
            .into_iter()
            .map(|session| (session.id.clone(), session))
            .collect::<BTreeMap<_, _>>();
        if !sessions.contains_key(session_id) {
            return Err(HarnessError::SessionNotFound(session_id.into()));
        }

        let mut ancestors = Vec::new();
        let mut cursor = sessions
            .get(session_id)
            .and_then(|session| session.parent_session_id.clone());
        let mut seen = BTreeSet::new();
        while let Some(parent_id) = cursor {
            if !seen.insert(parent_id.clone()) {
                break;
            }
            ancestors.push(parent_id.clone());
            cursor = sessions
                .get(&parent_id)
                .and_then(|session| session.parent_session_id.clone());
        }
        ancestors.reverse();

        let mut descendants = Vec::new();
        let mut queue = VecDeque::from([session_id.to_string()]);
        let mut visited = BTreeSet::from([session_id.to_string()]);
        while let Some(parent) = queue.pop_front() {
            for session in sessions.values() {
                if session.parent_session_id.as_deref() == Some(parent.as_str())
                    && visited.insert(session.id.clone())
                {
                    descendants.push(session.id.clone());
                    queue.push_back(session.id.clone());
                }
            }
        }

        Ok(SessionLineage {
            session_id: session_id.into(),
            ancestors,
            descendants,
        })
    }

    pub fn related_events(
        &self,
        session_id: &str,
        kind: Option<&str>,
        agent_id: Option<&str>,
        limit: usize,
    ) -> HarnessResult<Vec<HarnessEvent>> {
        let snapshot = self.harness.snapshot()?;
        let session = snapshot
            .sessions
            .into_iter()
            .find(|session| session.id == session_id)
            .ok_or_else(|| HarnessError::SessionNotFound(session_id.into()))?;
        let mut events = session
            .events
            .into_iter()
            .filter(|event| kind.is_none_or(|kind| event.kind == kind))
            .filter(|event| {
                agent_id.is_none_or(|agent_id| event.agent_id.as_deref() == Some(agent_id))
            })
            .collect::<Vec<_>>();
        events.sort_by_key(|event| std::cmp::Reverse(event.sequence));
        events.truncate(limit.clamp(1, 500));
        Ok(events)
    }

    pub fn suggest_title(&self, session_id: &str) -> HarnessResult<TitleCandidate> {
        let snapshot = self.harness.snapshot()?;
        let session = snapshot
            .sessions
            .into_iter()
            .find(|session| session.id == session_id)
            .ok_or_else(|| HarnessError::SessionNotFound(session_id.into()))?;
        if !session.title.trim().is_empty() {
            return Ok(TitleCandidate {
                session_id: session.id,
                title: session.title,
                source: "session".into(),
            });
        }
        let title = session
            .events
            .iter()
            .find(|event| event.kind == "user/message")
            .and_then(|event| event.payload.get("text"))
            .and_then(Value::as_str)
            .map(compact_title)
            .filter(|title| !title.is_empty())
            .unwrap_or_else(|| "Untitled session".into());
        Ok(TitleCandidate {
            session_id: session.id,
            title,
            source: "fallback".into(),
        })
    }

    pub fn record_telemetry(
        &self,
        name: impl Into<String>,
        attributes: Value,
    ) -> HarnessResult<TelemetryRecord> {
        let record = TelemetryRecord {
            id: format!("telemetry:{}", Uuid::new_v4()),
            name: required(&name.into(), "telemetry name")?,
            attributes,
            timestamp_ms: now_ms(),
        };
        let mut state = self.state()?;
        state.telemetry.push_back(record.clone());
        while state.telemetry.len() > 4096 {
            state.telemetry.pop_front();
        }
        Ok(record)
    }

    pub fn poll_telemetry(&self, limit: usize) -> HarnessResult<Vec<TelemetryRecord>> {
        let mut state = self.state()?;
        let count = limit.clamp(1, 500).min(state.telemetry.len());
        Ok(state.telemetry.drain(..count).collect())
    }

    pub fn snapshot(&self) -> HarnessResult<Value> {
        let state = self.state()?;
        serde_json::to_value(serde_json::json!({
            "permissionPresets": state.permission_presets.values().collect::<Vec<_>>(),
            "activePermissionPreset": state.active_permission_preset,
            "questions": state.questions.values().collect::<Vec<_>>(),
            "plans": state.plans.values().collect::<Vec<_>>(),
            "bundles": state.bundles.values().collect::<Vec<_>>(),
            "presets": state.presets.values().collect::<Vec<_>>(),
            "extensions": state.extensions.values().collect::<Vec<_>>(),
            "hooks": state.hooks.values().collect::<Vec<_>>(),
            "hookEmissions": state.hook_emissions,
            "telemetryBuffered": state.telemetry.len(),
        }))
        .map_err(|error| HarnessError::InvalidConfig(error.to_string()))
    }

    fn ensure_session(&self, session_id: &str) -> HarnessResult<()> {
        if self
            .harness
            .snapshot()?
            .sessions
            .iter()
            .any(|session| session.id == session_id)
        {
            Ok(())
        } else {
            Err(HarnessError::SessionNotFound(session_id.into()))
        }
    }

    fn state(&self) -> HarnessResult<MutexGuard<'_, AdvancedState>> {
        self.state.lock().map_err(|_| HarnessError::StatePoisoned)
    }
}

fn validate_question_answer(question: &UserQuestion, answer: &Value) -> HarnessResult<()> {
    if question.options.is_empty() || question.allow_freeform {
        return Ok(());
    }
    let selected = answer
        .as_str()
        .or_else(|| answer.get("optionId").and_then(Value::as_str))
        .ok_or_else(|| {
            HarnessError::InvalidConfig("question answer must select an option".into())
        })?;
    if question.options.iter().any(|option| option.id == selected) {
        Ok(())
    } else {
        Err(HarnessError::InvalidConfig(format!(
            "unknown question option {selected}"
        )))
    }
}

fn compact_title(text: &str) -> String {
    let compact = text.split_whitespace().collect::<Vec<_>>().join(" ");
    compact.chars().take(80).collect()
}

fn required(value: &str, label: &str) -> HarnessResult<String> {
    let value = value.trim();
    if value.is_empty() {
        Err(HarnessError::InvalidConfig(format!("{label} is required")))
    } else {
        Ok(value.to_string())
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
    use mahayana_core::BuildProfile;

    #[test]
    fn plan_exit_requires_review_when_configured() {
        let harness = MahayanaHarness::new(BuildProfile::DesktopFull);
        let session = harness.create_session("plan").unwrap();
        let services = AdvancedHarnessServices::new(harness);
        services
            .enter_plan("agent-1", session.id, vec!["inspect".into()], true)
            .unwrap();
        assert!(matches!(
            services.exit_plan("agent-1", false, None),
            Err(HarnessError::ApprovalRequired(_))
        ));
        assert_eq!(
            services
                .exit_plan("agent-1", true, Some("user".into()))
                .unwrap()
                .mode,
            "execution"
        );
    }

    #[test]
    fn lineage_tracks_parent_and_descendant_sessions() {
        let harness = MahayanaHarness::new(BuildProfile::DesktopFull);
        let root = harness.create_session("root").unwrap();
        let child = harness.fork_session(&root.id).unwrap();
        let grandchild = harness.fork_session(&child.id).unwrap();
        let services = AdvancedHarnessServices::new(harness);
        let lineage = services.session_lineage(&child.id).unwrap();
        assert_eq!(lineage.ancestors, vec![root.id]);
        assert_eq!(lineage.descendants, vec![grandchild.id]);
    }

    #[test]
    fn preset_resolution_merges_bundle_settings_in_order() {
        let harness = MahayanaHarness::new(BuildProfile::DesktopFull);
        let services = AdvancedHarnessServices::new(harness);
        services
            .register_bundle(BundleDefinition {
                id: "base".into(),
                services: vec![],
                plugins: vec![],
                settings: serde_json::json!({"model": "a", "temperature": 1}),
            })
            .unwrap();
        services
            .register_bundle(BundleDefinition {
                id: "override".into(),
                services: vec![],
                plugins: vec![],
                settings: serde_json::json!({"model": "b"}),
            })
            .unwrap();
        services
            .register_preset(PresetDefinition {
                id: "coding".into(),
                bundles: vec!["base".into(), "override".into()],
                prompt_sections: vec![],
                tool_allowlist: vec![],
                model: None,
                metadata: Value::Null,
            })
            .unwrap();
        let resolved = services.resolve_preset("coding").unwrap();
        assert_eq!(resolved.settings["model"], "b");
        assert_eq!(resolved.settings["temperature"], 1);
    }
}
