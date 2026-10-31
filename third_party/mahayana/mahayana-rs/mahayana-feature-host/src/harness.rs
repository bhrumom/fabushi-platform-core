//! Product-facing bridge for the Rust-native Mahayana harness.
//!
//! Presentation shells use this bridge instead of depending on harness internals.
//! It keeps the public surface JSON-friendly so Electron, Swift/Kotlin and CLI
//! hosts can share the same implementation without a JavaScript compatibility
//! layer.

use mahayana_core::{ApprovalDecision, BuildProfile, ConversationId};
use mahayana_harness::{
    AgentDescriptor, GoalRecord, HarnessEvent, HarnessResult, JobRecord, MahayanaHarness,
    PluginManifest, ProfileDefinition, RuntimeSnapshot, SessionRecord, WorkflowRecord,
};
use serde_json::Value;

#[derive(Clone)]
pub struct HarnessFeatureController {
    harness: MahayanaHarness,
}

impl HarnessFeatureController {
    pub fn new(build_profile: BuildProfile) -> Self {
        Self {
            harness: MahayanaHarness::new(build_profile),
        }
    }

    pub fn from_harness(harness: MahayanaHarness) -> Self {
        Self { harness }
    }

    pub fn inner(&self) -> &MahayanaHarness {
        &self.harness
    }

    pub fn snapshot(&self) -> HarnessResult<RuntimeSnapshot> {
        self.harness.snapshot()
    }

    pub fn dump_config(&self) -> HarnessResult<Value> {
        self.harness.dump_config()
    }

    pub fn poll_event(&self) -> HarnessResult<Option<HarnessEvent>> {
        self.harness.poll_event()
    }

    pub fn register_service(&self, service: impl Into<String>) -> HarnessResult<()> {
        self.harness.register_service(service)
    }

    pub fn register_profile(&self, profile: ProfileDefinition) -> HarnessResult<()> {
        self.harness.register_profile(profile)
    }

    pub fn activate_profile(&self, profile_id: &str) -> HarnessResult<()> {
        self.harness.activate_profile(profile_id)
    }

    pub fn mount_plugin(&self, manifest: PluginManifest) -> HarnessResult<()> {
        self.harness.mount_plugin(manifest)
    }

    pub fn unmount_plugin(&self, plugin_id: &str) -> HarnessResult<()> {
        self.harness.unmount_plugin(plugin_id)
    }

    pub fn create_session(&self, title: impl Into<String>) -> HarnessResult<SessionRecord> {
        self.harness.create_session(title)
    }

    pub fn ensure_conversation_session(
        &self,
        conversation_id: &ConversationId,
    ) -> HarnessResult<SessionRecord> {
        self.harness
            .ensure_session_for_conversation(conversation_id)
    }

    pub fn fork_session(&self, session_id: &str) -> HarnessResult<SessionRecord> {
        self.harness.fork_session(session_id)
    }

    pub fn spawn_agent(
        &self,
        name: impl Into<String>,
        preset: impl Into<String>,
        session_id: impl Into<String>,
    ) -> HarnessResult<AgentDescriptor> {
        self.harness.spawn_agent(name, preset, session_id)
    }

    pub fn set_goal(&self, session_id: &str, text: impl Into<String>) -> HarnessResult<GoalRecord> {
        self.harness.set_goal(session_id, text)
    }

    pub fn update_goal_status(
        &self,
        goal_id: &str,
        status: impl Into<String>,
    ) -> HarnessResult<GoalRecord> {
        self.harness.update_goal_status(goal_id, status)
    }

    pub fn create_job(&self, kind: impl Into<String>) -> HarnessResult<JobRecord> {
        self.harness.create_job(kind)
    }

    pub fn update_job(
        &self,
        job_id: &str,
        status: impl Into<String>,
        result_ref: Option<String>,
    ) -> HarnessResult<JobRecord> {
        self.harness.update_job(job_id, status, result_ref)
    }

    pub fn register_workflow(&self, workflow: WorkflowRecord) -> HarnessResult<()> {
        self.harness.register_workflow(workflow)
    }

    pub fn resolve_approval(
        &self,
        approval_id: &str,
        decision: ApprovalDecision,
    ) -> HarnessResult<()> {
        self.harness.resolve_approval(approval_id, decision)
    }

    pub fn append_session_event(
        &self,
        session_id: &str,
        kind: impl Into<String>,
        payload: Value,
    ) -> HarnessResult<HarnessEvent> {
        self.harness.append_session_event(session_id, kind, payload)
    }

    pub fn transcript(&self, session_id: &str) -> HarnessResult<String> {
        self.harness.transcript(session_id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bridge_exposes_shared_harness_state() {
        let controller = HarnessFeatureController::new(BuildProfile::DesktopFull);
        controller.register_service("tools").unwrap();
        let session = controller.create_session("bridge").unwrap();
        controller
            .append_session_event(
                &session.id,
                "user/message",
                serde_json::json!({"text": "hello"}),
            )
            .unwrap();
        assert_eq!(controller.transcript(&session.id).unwrap(), "user: hello");
        assert_eq!(controller.snapshot().unwrap().services, vec!["tools"]);
    }
}
