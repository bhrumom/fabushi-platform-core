use super::*;
use crate::automation_runs::{self as domain, Completion, PollRequest, RunEvent};

#[derive(Deserialize)]
pub(super) struct RunRow {
    event_json: String,
    event_digest: String,
    completion_json: Option<String>,
    pub(super) completion_digest: Option<String>,
    completed_at: Option<i64>,
    acknowledged_at: Option<i64>,
}
pub(super) async fn bounded_json<T: serde::de::DeserializeOwned>(request: &mut Request) -> Result<T> {
    if request.headers().get("Content-Length")?.and_then(|v| v.parse::<usize>().ok()).is_some_and(|v| v > 80_000) {
        return Err(worker::Error::RustError("automation request exceeds size limit".into()));
    }
    let body = request.text().await?;
    if body.len() > 80_000 {
        return Err(worker::Error::RustError("automation request exceeds size limit".into()));
    }
    serde_json::from_str(&body).map_err(internal_json)
}
pub(super) async fn read_run(database: &worker::D1Database, user: &str, run: &str) -> Result<Option<RunRow>> {
    worker::query!(database,
        "SELECT event_json,event_digest,completion_json,completion_digest,completed_at,acknowledged_at
         FROM automation_runs WHERE user_id=?1 AND run_uuid=?2", user, run)?
        .first::<RunRow>(None).await
}
fn internal_json(error: serde_json::Error) -> worker::Error {
    worker::Error::RustError(error.to_string())
}
fn response(value: Value) -> Result<Response> {
    Ok(Response::from_json(&value)?.with_headers(auth_headers()))
}
pub(super) fn run_value(user: &str, run: &str, row: &RunRow) -> Result<Value> {
    let event: RunEvent = serde_json::from_str(&row.event_json).map_err(internal_json)?;
    let completion = row.completion_json.as_deref().map(serde_json::from_str::<Value>)
        .transpose().map_err(internal_json)?;
    let receipt = row.completion_digest.as_deref().map(|d| domain::receipt(user,run,d))
        .transpose().map_err(internal_json)?;
    Ok(json!({"protocol":domain::PROTOCOL,"event":event.wire_event(),"completion":completion,
        "receipt":receipt,"completedAt":row.completed_at,"acknowledgedAt":row.acknowledged_at}))
}

pub(super) async fn automation_run_create(mut request: Request, context: RouteContext<()>) -> Result<Response> {
    let account = match authenticated_session_account(&request,&context.env).await {
        Ok(account)=>account, Err(_)=>return error_response(401,"unauthorized","An active Fabushi account session is required."),
    };
    let event: RunEvent = match bounded_json(&mut request).await {
        Ok(event)=>event, Err(_)=>return error_response(400,"invalid_automation_event","Invalid automation event."),
    };
    if !event.validate() { return error_response(400,"invalid_automation_event","Invalid automation event."); }
    let database=context.env.d1(DATABASE_BINDING)?;
    let event_json=serde_json::to_string(&event).map_err(internal_json)?;
    let event_digest=domain::digest(&event).map_err(internal_json)?;
    worker::query!(&database,domain::INSERT_RUN,&account.user_id,&event.run_uuid,&event_json,&event_digest,now_seconds())?
        .run().await?;
    let Some(row)=read_run(&database,&account.user_id,&event.run_uuid).await? else {
        return error_response(429,"automation_journal_capacity","Automation journal capacity reached.");
    };
    if row.event_digest!=event_digest {
        return error_response(409,"run_uuid_conflict","runUuid already identifies a different event.");
    }
    response(run_value(&account.user_id,&event.run_uuid,&row)?)
}

pub(super) async fn automation_run_get(request: Request, context: RouteContext<()>) -> Result<Response> {
    let account=match authenticated_session_account(&request,&context.env).await {
        Ok(account)=>account, Err(_)=>return error_response(401,"unauthorized","An active Fabushi account session is required."),
    };
    let run=context.param("run_uuid").map(String::as_str).unwrap_or_default();
    if !domain::valid_id(run) { return error_response(400,"invalid_run_uuid","Invalid runUuid."); }
    let database=context.env.d1(DATABASE_BINDING)?;
    let Some(row)=read_run(&database,&account.user_id,run).await? else {
        return error_response(404,"run_not_found","Automation run not found.");
    };
    response(run_value(&account.user_id,run,&row)?)
}

pub(super) async fn automation_run_complete(mut request: Request, context: RouteContext<()>) -> Result<Response> {
    let account=match authenticated_session_account(&request,&context.env).await {
        Ok(account)=>account, Err(_)=>return error_response(401,"unauthorized","An active Fabushi account session is required."),
    };
    let completion: Completion=match bounded_json(&mut request).await {
        Ok(value)=>value, Err(_)=>return error_response(400,"invalid_completion","Invalid completion."),
    };
    if !completion.validate() { return error_response(400,"invalid_completion","Invalid completion."); }
    let database=context.env.d1(DATABASE_BINDING)?;
    let completion_json=serde_json::to_string(&completion).map_err(internal_json)?;
    let completion_digest=domain::digest(&completion).map_err(internal_json)?;
    worker::query!(&database,domain::COMPLETE_RUN,&completion_json,&completion_digest,now_seconds(),&account.user_id,&completion.run_uuid)?
        .run().await?;
    let Some(row)=read_run(&database,&account.user_id,&completion.run_uuid).await? else {
        return error_response(404,"run_not_found","Automation run not found.");
    };
    if row.completion_digest.as_deref()!=Some(completion_digest.as_str()) {
        return error_response(409,"completion_conflict","A different completion is already recorded.");
    }
    response(run_value(&account.user_id,&completion.run_uuid,&row)?)
}

pub(super) async fn automation_events_poll(mut request: Request, context: RouteContext<()>) -> Result<Response> {
    let account=match authenticated_session_account(&request,&context.env).await {
        Ok(account)=>account, Err(_)=>return error_response(401,"unauthorized","An active Fabushi account session is required."),
    };
    let input: PollRequest=match bounded_json(&mut request).await {
        Ok(value)=>value, Err(_)=>return error_response(400,"invalid_poll","Invalid poll."),
    };
    if input.ack_run_uuids.len()>100 || input.ack_run_uuids.iter().any(|id| !domain::valid_id(id)) {
        return error_response(400,"invalid_ack","At most 100 valid run IDs may be acknowledged.");
    }
    let ids=input.ack_run_uuids.into_iter().collect::<std::collections::BTreeSet<_>>();
    let database=context.env.d1(DATABASE_BINDING)?;
    if !ids.is_empty() {
        #[derive(Deserialize)] struct Count { count: i64 }
        let ids_json=serde_json::to_string(&ids).map_err(internal_json)?;
        let count=worker::query!(&database,
            "SELECT COUNT(*) AS count FROM automation_runs WHERE user_id=?1
             AND completion_digest IS NOT NULL AND run_uuid IN (SELECT value FROM json_each(?2))",
            &account.user_id,&ids_json)?.first::<Count>(None).await?;
        if count.map(|v|v.count)!=Some(ids.len() as i64) {
            return error_response(409,"ack_not_completed","ACK requires a completed account-owned run.");
        }
        // Completion is immutable and rows are not deleted. After validation no concurrent
        // request can invalidate eligibility; the batch atomically acknowledges the set.
        let mut statements=Vec::with_capacity(ids.len());
        let now=now_seconds();
        for id in &ids {
            statements.push(worker::query!(&database,domain::ACK_RUN,now,&account.user_id,id)?);
        }
        database.batch(statements).await?;
    }
    #[derive(Deserialize)] struct EventRow { event_json: String }
    let rows=worker::query!(&database,
        "SELECT event_json FROM automation_runs WHERE user_id=?1 AND acknowledged_at IS NULL
         ORDER BY created_at ASC,run_uuid ASC LIMIT 100",&account.user_id)?
        .all().await?.results::<EventRow>()?;
    let mut events=Vec::with_capacity(rows.len());
    for row in rows {
        let event:RunEvent=serde_json::from_str(&row.event_json).map_err(internal_json)?;
        events.push(event.wire_event());
    }
    response(json!({"protocol":domain::PROTOCOL,"events":events,"nextPollAfterMs":4000}))
}
