use super::*;
use std::collections::HashMap;
use crate::automation_definitions::{self as domain,Definition,PutDefinition,DeleteDefinition};
use crate::automation_runs::{self as runs,RunEvent};

#[derive(Deserialize)]
struct DefinitionRow {
    automation_id:String,agent_id:String,definition_json:String,revision:String,mutation_id:String,
    next_runs_json:String,updated_at:i64,deleted_at:Option<i64>,
}
const SELECT_ROW:&str="SELECT automation_id,agent_id,definition_json,revision,mutation_id,next_runs_json,updated_at,deleted_at
 FROM automation_definitions WHERE user_id=?1 AND automation_id=?2";
async fn row(db:&worker::D1Database,user:&str,id:&str)->Result<Option<DefinitionRow>> {
    worker::query!(db,SELECT_ROW,user,id)?.first::<DefinitionRow>(None).await
}
fn json_error(error:serde_json::Error)->worker::Error {worker::Error::RustError(error.to_string())}
async fn body<T:serde::de::DeserializeOwned>(request:&mut Request)->Result<T> {
    if request.headers().get("Content-Length")?.and_then(|v|v.parse::<usize>().ok()).is_some_and(|n|n>80000) {
        return Err(worker::Error::RustError("automation request limit".into()));
    }
    let text=request.text().await?;
    if text.len()>80000 {return Err(worker::Error::RustError("automation request limit".into()));}
    serde_json::from_str(&text).map_err(json_error)
}
fn wire(row:&DefinitionRow)->Result<Value> {
    let definition=if row.deleted_at.is_some() {None} else {Some(serde_json::from_str::<Definition>(&row.definition_json).map_err(json_error)?)};
    Ok(json!({"id":row.automation_id,"agentId":row.agent_id,"definition":definition,"revision":row.revision,"deleted":row.deleted_at.is_some(),
        "updatedAt":row.updated_at,"nextRunsMs":serde_json::from_str::<Value>(&row.next_runs_json).map_err(json_error)?}))
}
async fn next_wake(db:&worker::D1Database,user:&str)->Result<Option<i64>> {
    #[derive(Deserialize)]struct Next{next_ms:Option<i64>}
    Ok(worker::query!(db,"SELECT MIN(CAST(p.value AS INTEGER)) AS next_ms FROM automation_definitions d,
        json_each(d.next_runs_json) p WHERE d.user_id=?1 AND d.deleted_at IS NULL AND p.type='integer'",user)?
        .first::<Next>(None).await?.and_then(|v|v.next_ms))
}
async fn response(db:&worker::D1Database,user:&str,value:Value)->Result<Response> {
    let headers=auth_headers();
    headers.set("x-fabushi-automation-clock-user",user)?;
    headers.set("x-fabushi-automation-clock-at",&next_wake(db,user).await?.unwrap_or(0).to_string())?;
    Ok(Response::from_json(&value)?.with_headers(headers))
}
fn id(context:&RouteContext<()>)->&str {context.param("automation_id").map(String::as_str).unwrap_or_default()}
pub(super) async fn automation_definition_get(request:Request,context:RouteContext<()>)->Result<Response> {
    let account=match authenticated_live_account(&request,&context.env).await {Ok(a)=>a,Err(_)=>return error_response(401,"unauthorized","An active ordinary account session is required.")};
    if !runs::valid_id(id(&context)){return error_response(400,"invalid_automation_id","Invalid automation ID.");}
    let db=context.env.d1(DATABASE_BINDING)?;
    let Some(found)=row(&db,&account.user_id,id(&context)).await? else{return error_response(404,"automation_not_found","Automation not found.");};
    response(&db,&account.user_id,json!({"protocol":domain::PROTOCOL,"record":wire(&found)?})).await
}
pub(super) async fn automation_definition_list(request:Request,context:RouteContext<()>)->Result<Response> {
    let account=match authenticated_live_account(&request,&context.env).await {Ok(a)=>a,Err(_)=>return error_response(401,"unauthorized","An active ordinary account session is required.")};
    let url=request.url()?;let query=url.query_pairs().collect::<HashMap<_,_>>();
    if query.keys().any(|key|!matches!(key.as_ref(),"agentId"|"cursor")) {return error_response(400,"invalid_query","Invalid automation query.");}
    let agent=query.get("agentId").map(|v|v.as_ref()).unwrap_or("");
    let cursor=query.get("cursor").map(|v|v.as_ref()).unwrap_or("");
    if (!agent.is_empty()&&!runs::valid_id(agent))||(!cursor.is_empty()&&!runs::valid_id(cursor)) {return error_response(400,"invalid_query","Invalid automation query.");}
    let db=context.env.d1(DATABASE_BINDING)?;
    let rows=worker::query!(&db,"SELECT automation_id,agent_id,definition_json,revision,mutation_id,next_runs_json,updated_at,deleted_at
        FROM automation_definitions WHERE user_id=?1 AND (?2='' OR agent_id=?2) AND automation_id>?3
        ORDER BY automation_id LIMIT 100",&account.user_id,agent,cursor)?.all().await?.results::<DefinitionRow>()?;
    let next_cursor=if rows.len()==100 {rows.last().map(|r|r.automation_id.clone())}else{None};
    let records=rows.iter().map(wire).collect::<Result<Vec<_>>>()?;
    response(&db,&account.user_id,json!({"protocol":domain::PROTOCOL,"records":records,"nextCursor":next_cursor})).await
}
pub(super) async fn automation_definition_put(mut request:Request,context:RouteContext<()>)->Result<Response> {
    let account=match authenticated_live_account(&request,&context.env).await {Ok(a)=>a,Err(_)=>return error_response(401,"unauthorized","An active ordinary account session is required.")};
    let input:PutDefinition=match body(&mut request).await {Ok(v)=>v,Err(_)=>return error_response(400,"invalid_definition","Invalid automation definition.")};
    if !input.validate()||input.definition.id!=id(&context) {return error_response(400,"invalid_definition","Invalid automation definition.");}
    let db=context.env.d1(DATABASE_BINDING)?;
    let mutation=input.mutation_id().map_err(json_error)?;
    if let Some(found)=row(&db,&account.user_id,id(&context)).await? {
        if found.mutation_id==mutation {return response(&db,&account.user_id,json!({"protocol":domain::PROTOCOL,"record":wire(&found)?})).await;}
    }
    let positions=input.definition.initial_positions(now_seconds()*1000).ok_or_else(||worker::Error::RustError("invalid schedule position".into()))?;
    let json=serde_json::to_string(&input.definition).map_err(json_error)?;
    let positions=serde_json::to_string(&positions).map_err(json_error)?;
    let revision=format!("definition-{}",Uuid::new_v4());
    worker::query!(&db,domain::WRITE_DEFINITION,&account.user_id,&input.definition.id,&input.definition.agent_id,
        &json,&revision,&mutation,&positions,now_seconds(),input.base_revision.as_deref())?.run().await?;
    let Some(found)=row(&db,&account.user_id,id(&context)).await? else {
        return error_response(if input.base_revision.is_some(){404}else{429},"definition_not_published","Automation not found or account definition capacity reached.");
    };
    if found.mutation_id!=mutation {return error_response(409,"definition_conflict","Automation changed; reconcile before writing.");}
    response(&db,&account.user_id,json!({"protocol":domain::PROTOCOL,"record":wire(&found)?})).await
}
pub(super) async fn automation_definition_delete(mut request:Request,context:RouteContext<()>)->Result<Response> {
    let account=match authenticated_live_account(&request,&context.env).await {Ok(a)=>a,Err(_)=>return error_response(401,"unauthorized","An active ordinary account session is required.")};
    let input:DeleteDefinition=match body(&mut request).await {Ok(v)=>v,Err(_)=>return error_response(400,"invalid_delete","Invalid automation delete.")};
    if !runs::valid_id(id(&context))||!runs::valid_id(&input.base_revision){return error_response(400,"invalid_delete","Invalid automation delete.");}
    let db=context.env.d1(DATABASE_BINDING)?;
    let mutation=runs::digest(&json!({"operation":"delete","id":id(&context),"baseRevision":input.base_revision})).map_err(json_error)?;
    let revision=format!("definition-{}",Uuid::new_v4());
    worker::query!(&db,"UPDATE automation_definitions SET revision=?1,mutation_id=?2,deleted_at=?3,updated_at=?3,next_runs_json='[]'
        WHERE user_id=?4 AND automation_id=?5 AND revision=?6 AND deleted_at IS NULL",
        &revision,&mutation,now_seconds(),&account.user_id,id(&context),&input.base_revision)?.run().await?;
    let Some(found)=row(&db,&account.user_id,id(&context)).await? else{return error_response(404,"automation_not_found","Automation not found.");};
    if found.mutation_id!=mutation {return error_response(409,"definition_conflict","Automation changed; reconcile before deleting.");}
    response(&db,&account.user_id,json!({"protocol":domain::PROTOCOL,"record":wire(&found)?})).await
}

#[derive(Deserialize)]#[serde(rename_all="camelCase",deny_unknown_fields)]struct Tick {user_id:String}
pub(super) async fn automation_clock_tick(mut request:Request,context:RouteContext<()>)->Result<Response> {
    let expected=context.env.var("__FABUSHI_CLOCK_CAPABILITY").ok().map(|v|v.to_string());
    let supplied=request.headers().get("x-fabushi-clock-proof")?;
    if !expected.as_deref().zip(supplied.as_deref()).is_some_and(|(a,b)|a.len()>=32&&constant_time_eq(a.as_bytes(),b.as_bytes())) {
        return error_response(401,"clock_capability_required","Internal clock capability required.");
    }
    let input:Tick=match body(&mut request).await{Ok(v)=>v,Err(_)=>return error_response(400,"invalid_clock_request","Invalid clock request.")};
    if !runs::valid_id(&input.user_id){return error_response(400,"invalid_clock_request","Invalid clock request.");}
    let value=tick_user(&context.env,&input.user_id).await?;
    Ok(Response::from_json(&value)?.with_headers(auth_headers()))
}
async fn tick_user(env:&Env,user:&str)->Result<Value> {
    let db=env.d1(DATABASE_BINDING)?;let now=now_seconds()*1000;let mut emitted=0;let mut blocked=false;
    // One account can have several due definitions. Rotate the entire due set,
    // including untouched siblings when the first occurrence hits account capacity.
    worker::query!(&db,"UPDATE automation_definitions SET clock_checked_at=?1
        WHERE user_id=?2 AND deleted_at IS NULL AND EXISTS(SELECT 1 FROM json_each(next_runs_json) p
        WHERE p.type='integer' AND CAST(p.value AS INTEGER)<=?3)",now_seconds(),user,now)?.run().await?;
    for _ in 0..100 {
        let Some(found)=worker::query!(&db,"SELECT automation_id,agent_id,definition_json,revision,mutation_id,next_runs_json,updated_at,deleted_at
            FROM automation_definitions d WHERE user_id=?1 AND deleted_at IS NULL
            AND EXISTS(SELECT 1 FROM json_each(d.next_runs_json) p WHERE p.type='integer' AND CAST(p.value AS INTEGER)<=?2)
            ORDER BY updated_at,automation_id LIMIT 1",user,now)?.first::<DefinitionRow>(None).await? else{break;};
        let definition:Definition=serde_json::from_str(&found.definition_json).map_err(json_error)?;
        let mut positions:Vec<Option<i64>>=serde_json::from_str(&found.next_runs_json).map_err(json_error)?;
        if !definition.enabled||!definition.validate()||positions.len()!=definition.workflow.triggers.len(){return Err(worker::Error::RustError("invalid persisted automation clock".into()));}
        let (index,due)=positions.iter().enumerate().filter_map(|(i,p)|p.map(|p|(i,p))).filter(|(_,p)|*p<=now).min_by_key(|(_,p)|*p)
            .ok_or_else(||worker::Error::RustError("invalid persisted due position".into()))?;
        let schedules=definition.schedules().ok_or_else(||worker::Error::RustError("invalid persisted schedule".into()))?;
        let schedule=schedules[index].as_deref().ok_or_else(||worker::Error::RustError("non-clock trigger has clock position".into()))?;
        positions[index]=Some(crate::automation_schedule::next_run(schedule,due).ok_or_else(||worker::Error::RustError("native schedule has no next occurrence".into()))?);
        let run=domain::scheduled_run_id(user,&definition.id,&found.revision,index,due).map_err(json_error)?;
        let event=RunEvent {run_uuid:run.clone(),sand_agent_id:definition.agent_id,automation_id:definition.id,
            timestamp_ms:now as u64,definition_revision:Some(found.revision.clone()),scheduled_for_ms:Some(due as u64),
            event:Some(json!({"type":"cron","triggerIndex":index}))};
        // Timestamp belongs to the occurrence, making duplicate alarms byte-identical.
        let mut event=event;event.timestamp_ms=due as u64;
        let event_json=serde_json::to_string(&event).map_err(json_error)?;let digest=runs::digest(&event).map_err(json_error)?;
        let next_json=serde_json::to_string(&positions).map_err(json_error)?;
        db.batch(vec![worker::query!(&db,domain::INSERT_SCHEDULED_RUN,user,&run,&event_json,&digest,now_seconds(),
            &event.automation_id,&found.revision,&found.next_runs_json)?,
            worker::query!(&db,domain::ADVANCE_CLOCK,&next_json,user,&event.automation_id,&found.revision,&found.next_runs_json,&run,&digest,now_seconds())?]).await?;
        let current=row(&db,user,&event.automation_id).await?;
        if current.as_ref().is_some_and(|v|v.revision==found.revision&&v.next_runs_json==found.next_runs_json){
            // Rotate a capacity-blocked account without consuming its due occurrence.
            worker::query!(&db,"UPDATE automation_definitions SET clock_checked_at=?1
                WHERE user_id=?2 AND automation_id=?3 AND revision=?4 AND next_runs_json=?5",
                now_seconds(),user,&event.automation_id,&found.revision,&found.next_runs_json)?.run().await?;
            blocked=true;break;
        }
        emitted+=1;
    }
    let next=next_wake(&db,user).await?;
    Ok(json!({"protocol":domain::PROTOCOL,"emitted":emitted,"capacityBlocked":blocked,
        "nextWakeAtMs":if blocked{Some(now+30000)}else{next}}))
}
pub(super) async fn reconcile_automation_clocks(env:&Env)->Result<()> {
    #[derive(Deserialize)]struct User{user_id:String}
    let db=env.d1(DATABASE_BINDING)?;
    let users=worker::query!(&db,"SELECT d.user_id FROM automation_definitions d,json_each(d.next_runs_json) p
        WHERE d.deleted_at IS NULL AND p.type='integer' AND CAST(p.value AS INTEGER)<=?1 GROUP BY d.user_id ORDER BY MIN(d.clock_checked_at),d.user_id LIMIT 50",now_seconds()*1000)?
        .all().await?.results::<User>()?;
    for user in users {tick_user(env,&user.user_id).await?;}
    Ok(())
}
