//! Native automation journal contract. Delivery is at-least-once, not an execution lease.
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use mahayana_platform_core::canonical_json_sha256;

pub const PROTOCOL: &str = "fabushi.automation.v1";
pub const INSERT_RUN: &str = "INSERT OR IGNORE INTO automation_runs
(user_id,run_uuid,event_json,event_digest,created_at)
SELECT ?1,?2,?3,?4,?5 WHERE
(SELECT COUNT(*) FROM automation_runs WHERE user_id=?1) < 20000";
pub const COMPLETE_RUN: &str = "UPDATE automation_runs SET completion_json=?1,completion_digest=?2,completed_at=?3
WHERE user_id=?4 AND run_uuid=?5 AND completion_digest IS NULL";
pub const ACK_RUN: &str = "UPDATE automation_runs SET acknowledged_at=COALESCE(acknowledged_at,?1)
WHERE user_id=?2 AND run_uuid=?3 AND completion_digest IS NOT NULL";

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RunEvent {
    pub run_uuid: String,
    pub sand_agent_id: String,
    pub automation_id: String,
    pub timestamp_ms: u64,
    pub definition_revision: Option<String>,
    pub scheduled_for_ms: Option<u64>,
    pub event: Option<Value>,
}
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Completion {
    pub run_uuid: String,
    pub status: String,
    pub error_message: Option<String>,
}
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PollRequest {
    #[serde(default)]
    pub ack_run_uuids: Vec<String>,
}
pub fn valid_id(value: &str) -> bool {
    !value.is_empty() && value.len() <= 200 && value.bytes().all(|b| b.is_ascii_alphanumeric() || b"._:-".contains(&b))
}
impl RunEvent {
    pub fn validate(&self) -> bool {
        valid_id(&self.run_uuid) && valid_id(&self.sand_agent_id) && valid_id(&self.automation_id)
            && self.timestamp_ms <= 9_007_199_254_740_991
            && self.scheduled_for_ms.is_none_or(|v| v <= 9_007_199_254_740_991)
            && self.definition_revision.as_ref().is_none_or(|v| valid_id(v))
            && serde_json::to_vec(self).is_ok_and(|v| v.len() <= 65_536)
    }
    pub fn wire_event(&self) -> Value {
        json!({"id": self.run_uuid, "sandAgentId": self.sand_agent_id,
            "automationId": self.automation_id, "timestampMs": self.timestamp_ms,
            "definitionRevision": self.definition_revision, "scheduledForMs": self.scheduled_for_ms,
            "event": self.event})
    }
}
impl Completion {
    pub fn validate(&self) -> bool {
        valid_id(&self.run_uuid) && matches!(self.status.as_str(), "succeeded" | "failed")
            && self.error_message.as_ref().is_none_or(|v| v.len() <= 4000)
            && (self.status != "succeeded" || self.error_message.is_none())
    }
}
pub fn digest<T: Serialize>(value: &T) -> Result<String, serde_json::Error> {
    canonical_json_sha256(&serde_json::to_value(value)?)
}
pub fn receipt(user_id: &str, run_uuid: &str, completion_digest: &str) -> Result<String, serde_json::Error> {
    canonical_json_sha256(&json!({"protocol":PROTOCOL,"userId":user_id,"runUuid":run_uuid,"completionDigest":completion_digest}))
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn validates_identity_and_completion_without_coercion() {
        for id in ["", "../run", " run", "a/b", "a\n"] { assert!(!valid_id(id)); }
        assert!(valid_id("run-1:device.a"));
        let mut value = Completion { run_uuid:"run-1".into(),status:"succeeded".into(),error_message:None };
        assert!(value.validate());
        value.error_message=Some("error".into()); assert!(!value.validate());
        value.status="unknown".into(); assert!(!value.validate());
        value.status="failed".into(); assert!(value.validate());
    }
    #[test]
    fn canonical_retry_receipt_is_stable_and_account_bound() {
        let a = json!({"status":"failed","event":{"b":2,"a":1}});
        let b = json!({"event":{"a":1,"b":2},"status":"failed"});
        assert_eq!(digest(&a).unwrap(),digest(&b).unwrap());
        let d=digest(&a).unwrap();
        assert_eq!(receipt("a","r",&d).unwrap(),receipt("a","r",&d).unwrap());
        assert_ne!(receipt("a","r",&d).unwrap(),receipt("b","r",&d).unwrap());
        assert_ne!(d,digest(&json!({"status":"succeeded"})).unwrap());
    }
}
