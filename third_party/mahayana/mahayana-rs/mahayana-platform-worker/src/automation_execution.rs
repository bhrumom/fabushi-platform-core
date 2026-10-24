//! Execution arbitration in the canonical automation journal, not a task engine.
use serde::Deserialize;
use crate::automation_runs::{valid_id,Completion};
pub const PROTOCOL:&str="fabushi.automation.execution.v1";
pub const CLAIM:&str="UPDATE automation_runs SET execution_state='reserved',claim_id=?1,
 claim_token_digest=?2,claim_device_id=?3,claim_session_id=?4,claim_generation=claim_generation+1,claim_expires_at=?5
 WHERE user_id=?6 AND run_uuid=?7 AND completion_digest IS NULL
 AND (execution_state IS NULL OR (execution_state='reserved' AND claim_expires_at<=?8 AND claim_id<>?1))
 AND EXISTS(SELECT 1 FROM remote_computers WHERE user_id=?6 AND device_id=?3 AND revoked_at IS NULL)
 AND (json_extract(event_json,'$.definitionRevision') IS NULL OR EXISTS(
 SELECT 1 FROM automation_definitions d WHERE d.user_id=?6
 AND d.automation_id=json_extract(event_json,'$.automationId')
 AND d.agent_id=json_extract(event_json,'$.sandAgentId')
 AND d.revision=json_extract(event_json,'$.definitionRevision') AND d.deleted_at IS NULL
 AND json_extract(d.definition_json,'$.enabled')=1))";
pub const START:&str="UPDATE automation_runs SET execution_state='running',execution_started_at=?1
 WHERE user_id=?2 AND run_uuid=?3 AND claim_id=?4 AND claim_token_digest=?5 AND claim_device_id=?6
 AND claim_generation=?7 AND execution_state='reserved' AND claim_expires_at>?1 AND completion_digest IS NULL
 AND EXISTS(SELECT 1 FROM remote_computers WHERE user_id=?2 AND device_id=?6 AND revoked_at IS NULL)
 AND (json_extract(event_json,'$.definitionRevision') IS NULL OR EXISTS(
 SELECT 1 FROM automation_definitions d WHERE d.user_id=?2
 AND d.automation_id=json_extract(event_json,'$.automationId')
 AND d.agent_id=json_extract(event_json,'$.sandAgentId')
 AND d.revision=json_extract(event_json,'$.definitionRevision') AND d.deleted_at IS NULL
 AND json_extract(d.definition_json,'$.enabled')=1))";
pub const UNKNOWN:&str="UPDATE automation_runs SET execution_state='outcome_unknown'
 WHERE user_id=?1 AND run_uuid=?2 AND claim_id=?3 AND claim_token_digest=?4 AND claim_device_id=?5
 AND claim_generation=?6 AND execution_state='running' AND completion_digest IS NULL
 AND EXISTS(SELECT 1 FROM remote_computers WHERE user_id=?1 AND device_id=?5 AND revoked_at IS NULL)";
pub const COMPLETE:&str="UPDATE automation_runs SET completion_json=?1,completion_digest=?2,completed_at=?3
 WHERE user_id=?4 AND run_uuid=?5 AND claim_id=?6 AND claim_token_digest=?7 AND claim_device_id=?8
 AND claim_generation=?9 AND execution_state IN ('running','outcome_unknown') AND completion_digest IS NULL
 AND EXISTS(SELECT 1 FROM remote_computers WHERE user_id=?4 AND device_id=?8 AND revoked_at IS NULL)";
#[derive(Deserialize)]
#[serde(rename_all="camelCase",deny_unknown_fields)]
pub struct Proof {pub claim_id:String,pub claim_token:String,pub device_secret:String,pub generation:Option<u64>}
impl Proof {
 pub fn validate(&self)->bool {valid_id(&self.claim_id)&&self.claim_token.len()>=32&&self.claim_token.len()<=128
  &&self.claim_token.bytes().all(|b|b.is_ascii_alphanumeric()||b"._:-".contains(&b))
  &&(48..=256).contains(&self.device_secret.len())}
 pub fn digest(&self)->Result<String,serde_json::Error>{crate::automation_runs::digest(&self.claim_token)}
}
#[derive(Deserialize)]
#[serde(rename_all="camelCase",deny_unknown_fields)]
pub struct Finish {pub proof:Proof,pub status:String,pub error_message:Option<String>}
impl Finish {
 pub fn completion(&self,run:&str)->Completion {Completion{run_uuid:run.into(),status:self.status.clone(),error_message:self.error_message.clone()}}
}
