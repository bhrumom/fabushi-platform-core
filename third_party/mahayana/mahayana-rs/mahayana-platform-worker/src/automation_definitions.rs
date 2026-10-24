//! Account definition/CAS and durable cloud-clock contract, separate from execution.
use serde::{Deserialize, Serialize};
use serde_json::{Value,json};
use crate::{automation_runs::{digest,valid_id},automation_schedule::next_run};

pub const PROTOCOL:&str="fabushi.automation.definition.v1";
pub const WRITE_DEFINITION:&str="INSERT INTO automation_definitions
 (user_id,automation_id,agent_id,definition_json,revision,mutation_id,next_runs_json,updated_at,deleted_at)
 SELECT ?1,?2,?3,?4,?5,?6,?7,?8,NULL WHERE (?9 IS NULL OR
 EXISTS(SELECT 1 FROM automation_definitions WHERE user_id=?1 AND automation_id=?2)) AND (
 EXISTS(SELECT 1 FROM automation_definitions WHERE user_id=?1 AND automation_id=?2)
 OR ((SELECT COUNT(*) FROM automation_definitions WHERE user_id=?1)<20000
 AND (SELECT COUNT(*) FROM automation_definitions WHERE user_id=?1 AND deleted_at IS NULL)<1000))
 ON CONFLICT(user_id,automation_id) DO UPDATE SET definition_json=excluded.definition_json,
 revision=excluded.revision,mutation_id=excluded.mutation_id,next_runs_json=excluded.next_runs_json,
 updated_at=excluded.updated_at,deleted_at=NULL
 WHERE automation_definitions.revision=?9 AND automation_definitions.agent_id=excluded.agent_id
 AND (automation_definitions.deleted_at IS NULL OR
 (SELECT COUNT(*) FROM automation_definitions WHERE user_id=?1 AND deleted_at IS NULL)<1000)";
pub const INSERT_SCHEDULED_RUN:&str="INSERT OR IGNORE INTO automation_runs
 (user_id,run_uuid,event_json,event_digest,created_at) SELECT ?1,?2,?3,?4,?5
 WHERE EXISTS(SELECT 1 FROM automation_definitions WHERE user_id=?1 AND automation_id=?6
 AND revision=?7 AND deleted_at IS NULL AND next_runs_json=?8)
 AND (SELECT COUNT(*) FROM automation_runs WHERE user_id=?1)<20000";
pub const ADVANCE_CLOCK:&str="UPDATE automation_definitions SET next_runs_json=?1
 WHERE user_id=?2 AND automation_id=?3 AND revision=?4 AND deleted_at IS NULL AND next_runs_json=?5
 AND EXISTS(SELECT 1 FROM automation_runs WHERE user_id=?2 AND run_uuid=?6 AND event_digest=?7)";

#[derive(Debug,Clone,Deserialize,Serialize)]
#[serde(rename_all="camelCase",deny_unknown_fields)]
pub struct Definition {
    pub id:String, pub agent_id:String, pub name:String, pub description:Option<String>,
    pub enabled:bool, pub workflow:Workflow,
}
#[derive(Debug,Clone,Deserialize,Serialize)]
#[serde(deny_unknown_fields)]
pub struct Workflow { pub triggers:Vec<Value>,pub prompts:Vec<Prompt> }
#[derive(Debug,Clone,Deserialize,Serialize)]
#[serde(deny_unknown_fields)]
pub struct Prompt { pub prompt:String }
#[derive(Debug,Deserialize,Serialize)]
#[serde(rename_all="camelCase",deny_unknown_fields)]
pub struct PutDefinition {
    pub definition:Definition,
    pub expect_absent:Option<bool>,
    pub base_revision:Option<String>,
}
#[derive(Debug,Deserialize,Serialize)]
#[serde(rename_all="camelCase",deny_unknown_fields)]
pub struct DeleteDefinition {pub base_revision:String}

impl Definition {
    pub fn schedules(&self)->Option<Vec<Option<String>>> {
        if self.workflow.triggers.is_empty() || self.workflow.triggers.len()>32 {return None;}
        let mut schedules=Vec::new();
        for trigger in &self.workflow.triggers {
            let outer=trigger.as_object()?;
            if outer.len()!=1 {return None;}
            let variant=outer.get("trigger")?.as_object()?;
            if variant.len()!=1 {return None;}
            let (kind,config)=variant.iter().next()?;
            if !config.is_object() {return None;}
            if kind=="Cron" {
                let config=config.as_object()?;
                if config.len()!=1 {return None;}
                let cron=config.get("cron")?.as_str()?;
                if cron.len()>512 || next_run(cron,0).is_none() {return None;}
                schedules.push(Some(cron.to_owned()));
            } else if ["Git","SlackTrigger","Linear","Pagerduty","Sentry","MicrosoftTeamsTrigger",
                "SlackReactionAdded","SlackMention","SlackAnyReactionAdded"].contains(&kind.as_str()) {
                schedules.push(None);
            } else {return None;}
        }
        Some(schedules)
    }
    pub fn validate(&self)->bool {
        valid_id(&self.id)&&valid_id(&self.agent_id)&&!self.name.trim().is_empty()&&self.name.len()<=512
        &&self.description.as_ref().is_none_or(|v|v.len()<=4000)
        &&!self.workflow.prompts.is_empty()&&self.workflow.prompts.len()<=10
        &&self.workflow.prompts.iter().all(|p|p.prompt.len()<=16384)
        &&serde_json::to_vec(self).is_ok_and(|v|v.len()<=65536)&&self.schedules().is_some()
    }
    pub fn initial_positions(&self,now:i64)->Option<Vec<Option<i64>>> {
        self.schedules()?.into_iter().map(|schedule| match schedule {
            Some(schedule) if self.enabled=>Some(Some(next_run(&schedule,now)?)), _=>Some(None),
        }).collect()
    }
}
impl PutDefinition {
    pub fn validate(&self)->bool {
        self.definition.validate() && match (self.expect_absent,self.base_revision.as_deref()) {
            (Some(true),None)=>true, (None|Some(false),Some(revision))=>valid_id(revision), _=>false,
        }
    }
    pub fn mutation_id(&self)->Result<String,serde_json::Error>{digest(self)}
}
pub fn scheduled_run_id(user:&str,id:&str,revision:&str,index:usize,due:i64)->Result<String,serde_json::Error>{
    Ok(format!("scheduled-{}",digest(&json!({"user":user,"id":id,"revision":revision,"triggerIndex":index,"due":due}))?))
}

#[cfg(test)] mod tests {
    use super::*;
    fn input()->Value {json!({"definition":{"id":"routine-1","agentId":"agent-1","name":"Daily","description":null,"enabled":true,
        "workflow":{"triggers":[{"trigger":{"Cron":{"cron":"CRON_TZ=Asia/Shanghai @daily"}}}],"prompts":[{"prompt":"run"}]}},"expectAbsent":true})}
    #[test] fn strict_definitions_and_clock_positions_are_bound_to_content() {
        let a:PutDefinition=serde_json::from_value(input()).unwrap();assert!(a.validate());
        assert_eq!(a.definition.initial_positions(0),Some(vec![Some(57600000)]));
        let mut b=input();b["baseRevision"]=json!("old");assert!(!serde_json::from_value::<PutDefinition>(b).unwrap().validate());
        let mut b=input();b["definition"]["workflow"]["triggers"][0]["trigger"]["Cron"]["cron"]=json!("@every 0s");assert!(!serde_json::from_value::<PutDefinition>(b).unwrap().validate());
        let mut b=input();b["definition"]["unknown"]=json!(true);assert!(serde_json::from_value::<PutDefinition>(b).is_err());
        assert_ne!(scheduled_run_id("a","x","r",0,1000).unwrap(),scheduled_run_id("b","x","r",0,1000).unwrap());
        assert_ne!(scheduled_run_id("a","x","r",0,1000).unwrap(),scheduled_run_id("a","x","r",1,1000).unwrap());
    }
}
