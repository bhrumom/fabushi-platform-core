use super::*;
use crate::automation_execution::{self as execution,Proof,Finish};
use crate::automation_runs as runs;

#[derive(Deserialize)]
struct ExecutionRow {
 execution_state:Option<String>,claim_id:Option<String>,claim_token_digest:Option<String>,
 claim_device_id:Option<String>,claim_generation:i64,claim_expires_at:Option<i64>,execution_started_at:Option<i64>,
}
async fn read(db:&worker::D1Database,user:&str,run:&str)->Result<Option<ExecutionRow>> {
 worker::query!(db,"SELECT execution_state,claim_id,claim_token_digest,claim_device_id,
 claim_generation,claim_expires_at,execution_started_at FROM automation_runs WHERE user_id=?1 AND run_uuid=?2",user,run)?
 .first::<ExecutionRow>(None).await
}
fn owns(row:&ExecutionRow,proof:&Proof,device:&str,digest:&str)->bool {
 row.claim_id.as_deref()==Some(proof.claim_id.as_str())&&row.claim_device_id.as_deref()==Some(device)
 &&row.claim_token_digest.as_deref().is_some_and(|stored|constant_time_eq(stored.as_bytes(),digest.as_bytes()))
}
async fn output(db:&worker::D1Database,user:&str,run:&str,row:&ExecutionRow)->Result<Response> {
 let journal=automations::read_run(db,user,run).await?.ok_or_else(||worker::Error::RustError("execution journal disappeared".into()))?;
 let value=automations::run_value(user,run,&journal)?;
 Ok(Response::from_json(&json!({"protocol":execution::PROTOCOL,"runUuid":run,"state":row.execution_state,
  "deviceId":row.claim_device_id,"claimId":row.claim_id,"generation":row.claim_generation,
  "reservedUntil":row.claim_expires_at,"startedAt":row.execution_started_at,"run":value}))?.with_headers(auth_headers()))
}
pub(super) async fn automation_execution_action(mut request:Request,context:RouteContext<()>)->Result<Response> {
 let account=match authenticated_live_account(&request,&context.env).await {Ok(a)=>a,Err(_)=>return error_response(401,"unauthorized","An active ordinary account session is required.")};
 let run=context.param("run_uuid").map(String::as_str).unwrap_or("");
 let action=context.param("action").map(String::as_str).unwrap_or("");
 if !runs::valid_id(run)||!matches!(action,"claim"|"start"|"unknown"|"complete"|"reconcile") {
  return error_response(400,"invalid_execution_action","Invalid execution action.");
 }
 let value:Value=match automations::bounded_json(&mut request).await {Ok(v)=>v,Err(_)=>return error_response(400,"invalid_execution_request","Invalid execution request.")};
 let finish=if action=="complete" {match serde_json::from_value::<Finish>(value.clone()){Ok(v)=>Some(v),Err(_)=>return error_response(400,"invalid_execution_request","Invalid execution request.")}}else{None};
 let proof=if let Some(ref finish)=finish {&finish.proof}else {
  // Keep the typed proof alive without accepting completion fields on other actions.
  return execution_proof_action(value,&account,run,action,&context.env).await;
 };
 let completion=finish.as_ref().unwrap().completion(run);
 if !proof.validate()||proof.generation.is_none_or(|v|v==0||v>i64::MAX as u64)||!completion.validate() {
  return error_response(400,"invalid_execution_request","Invalid execution request.");
 }
 let Some(device)=authorized_device(&context.env,&account,&proof.device_secret).await? else{return error_response(403,"execution_device_forbidden","An active registered session device is required.");};
 let db=context.env.d1(DATABASE_BINDING)?;
 let digest=proof.digest().map_err(json_error)?;
 let Some(row)=read(&db,&account.user_id,run).await? else{return error_response(404,"run_not_found","Automation run not found.");};
 if !owns(&row,proof,&device,&digest)||proof.generation!=Some(row.claim_generation as u64) {
  return error_response(409,"execution_claim_conflict","Reconcile the existing execution owner.");
 }
 let encoded=serde_json::to_string(&completion).map_err(json_error)?;
 let completion_digest=runs::digest(&completion).map_err(json_error)?;
 worker::query!(&db,execution::COMPLETE,&encoded,&completion_digest,now_seconds(),&account.user_id,run,
  &proof.claim_id,&digest,&device,row.claim_generation)?.run().await?;
 let journal=automations::read_run(&db,&account.user_id,run).await?.ok_or_else(||worker::Error::RustError("execution journal disappeared".into()))?;
 if journal.completion_digest.as_deref()!=Some(completion_digest.as_str()) {
  return error_response(409,"execution_completion_conflict","Execution is not started or a different completion is recorded.");
 }
 output(&db,&account.user_id,run,&row).await
}
fn json_error(error:serde_json::Error)->worker::Error {worker::Error::RustError(error.to_string())}
async fn authorized_device(env:&Env,account:&AuthenticatedAccount,secret:&str)->Result<Option<String>> {
 #[derive(Deserialize)]struct Device{device_id:String}
 let db=env.d1(ACCOUNT_DATABASE_BINDING)?;
 let Some(session)=worker::query!(&db,"SELECT device_id FROM account_sessions
  WHERE session_id=?1 AND user_id=?2 AND revoked_at IS NULL AND expires_at>?3",
  account.session_id.as_deref(),&account.user_id,now_seconds())?.first::<Device>(None).await? else{return Ok(None);};
 let platform=env.d1(DATABASE_BINDING)?;
 if !remote_computer::remote_desktop_secret_matches(&platform,&account.user_id,&session.device_id,secret).await? {return Ok(None);}
 Ok(Some(session.device_id))
}
async fn execution_proof_action(value:Value,account:&AuthenticatedAccount,run:&str,action:&str,env:&Env)->Result<Response> {
 let proof:Proof=match serde_json::from_value(value){Ok(v)=>v,Err(_)=>return error_response(400,"invalid_execution_request","Invalid execution request.")};
 if !proof.validate()||(action=="claim"&&proof.generation.is_some())
  ||(action!="claim"&&proof.generation.is_none_or(|v|v==0||v>i64::MAX as u64)) {
  return error_response(400,"invalid_execution_request","Invalid execution request.");
 }
 let Some(device)=authorized_device(env,account,&proof.device_secret).await? else{return error_response(403,"execution_device_forbidden","An active registered session device is required.");};
 let db=env.d1(DATABASE_BINDING)?;
 let digest=proof.digest().map_err(json_error)?;
 let Some(mut row)=read(&db,&account.user_id,run).await? else{return error_response(404,"run_not_found","Automation run not found.");};
 if action=="claim" {
  if !owns(&row,&proof,&device,&digest) {
   worker::query!(&db,execution::CLAIM,&proof.claim_id,&digest,&device,account.session_id.as_deref(),
    now_seconds()+60,&account.user_id,run,now_seconds())?.run().await?;
   row=read(&db,&account.user_id,run).await?.ok_or_else(||worker::Error::RustError("execution journal disappeared".into()))?;
  }
 } else if owns(&row,&proof,&device,&digest)&&proof.generation==Some(row.claim_generation as u64) {
  match action {
   "start"=>{worker::query!(&db,execution::START,now_seconds(),&account.user_id,run,&proof.claim_id,&digest,&device,row.claim_generation)?.run().await?;},
   "unknown"=>{worker::query!(&db,execution::UNKNOWN,&account.user_id,run,&proof.claim_id,&digest,&device,row.claim_generation)?.run().await?;},
   _=>{},
  }
  row=read(&db,&account.user_id,run).await?.ok_or_else(||worker::Error::RustError("execution journal disappeared".into()))?;
 }
 if !owns(&row,&proof,&device,&digest)||(action!="claim"&&proof.generation!=Some(row.claim_generation as u64))
  ||(action=="start"&&row.execution_state.as_deref()!=Some("running"))
  ||(action=="unknown"&&row.execution_state.as_deref()!=Some("outcome_unknown")) {
  return error_response(409,"execution_claim_conflict","Execution owner or state changed; reconcile before execution.");
 }
 output(&db,&account.user_id,run,&row).await
}
