use super::*;
use crate::mcp_oauth::{connector, digest, open, seal};
use worker::{Headers, RequestInit, RequestRedirect};

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Start { plugin_id: String }
#[derive(Deserialize)]
struct Attempt {
    attempt_id: String, user_id: String, session_id: String, plugin_id: String,
    state_hash: String, ticket_hash: String, verifier: String, status: String,
    payload_ciphertext: Option<String>, expires_at: i64,
}
#[derive(Deserialize)]
struct Principal { principal_id: String }
#[derive(Deserialize)]
struct Connection {
    connection_id: String, plugin_id: String, access_digest: String,
    refresh_digest: Option<String>, version: i64,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Refresh { refresh_token: String }
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Revoke { token: String, #[serde(default)] refresh_token: Option<String> }

fn broker_error() -> worker::Error { worker::Error::RustError("service authorization failed".into()) }
fn callback_url() -> &'static str { "https://api.ombhrum.com/api/mcp/oauth/callback" }
fn connection_provider(env: &Env, plugin_id: &str) -> Option<OAuthProviderConfig> {
    let (kind, scopes) = connector(plugin_id)?;
    // Dedicated connection credentials can be registered independently of login.
    let prefix = if kind == "google" { "MCP_GOOGLE" } else { "MCP_GITHUB" };
    let read = |suffix: &str| env.secret(&format!("{prefix}_{suffix}")).ok().map(|s| s.to_string())
        .or_else(|| env.var(&format!("{prefix}_{suffix}")).ok().map(|s| s.to_string())).filter(|s| !s.is_empty());
    let dedicated_id = read("CLIENT_ID");
    let dedicated_secret = read("CLIENT_SECRET");
    if dedicated_id.is_some() != dedicated_secret.is_some() { return None; }
    let mut provider = configured_provider(env, kind).or_else(|| {
        let client_id = read("CLIENT_ID")?;
        let client_secret = read("CLIENT_SECRET")?;
        Some(OAuthProviderConfig {
            id: if kind == "google" { "google" } else { "github" },
            display_name: if kind == "google" { "Google" } else { "GitHub" },
            issuer: if kind == "google" { "https://accounts.google.com" } else { "https://github.com" },
            kind: if kind == "google" { crate::identity_auth::ProviderKind::Oidc } else { crate::identity_auth::ProviderKind::Github },
            authorization_endpoint: if kind == "google" { "https://accounts.google.com/o/oauth2/v2/auth" } else { "https://github.com/login/oauth/authorize" },
            token_endpoint: if kind == "google" { "https://oauth2.googleapis.com/token" } else { "https://github.com/login/oauth/access_token" },
            userinfo_endpoint: if kind == "google" { "https://openidconnect.googleapis.com/v1/userinfo" } else { "https://api.github.com/user" },
            scopes, client_id, client_secret: Some(client_secret),
        })
    })?;
    if let Some(value) = read("CLIENT_ID") { provider.client_id = value; }
    if let Some(value) = read("CLIENT_SECRET") { provider.client_secret = Some(value); }
    provider.scopes = scopes;
    Some(provider)
}

pub(super) async fn mcp_cleanup(env: &Env) -> Result<()> {
    let db = env.d1(ACCOUNT_DATABASE_BINDING)?;
    let expired = worker::query!(&db,"SELECT * FROM account_mcp_oauth_attempts WHERE expires_at<=?1 AND status='ready' LIMIT 100",now_seconds())?.all().await?.results::<Attempt>()?;
    for row in expired {
        // An unclaimed completed grant must not outlive its delivery attempt.
        if let (Ok(key),Some(ciphertext))=(env.secret("ACCESS_TOKEN_PRIVATE_KEY_PEM"),row.payload_ciphertext.as_deref()) {
            if let Ok(plaintext)=open(&key.to_string(),&row.attempt_id,ciphertext) {
                if let Ok(value)=serde_json::from_slice::<Value>(&plaintext) {
                    if let Some(id)=value["connectionId"].as_str() {
                        worker::query!(&db,"UPDATE account_connections SET status='revoked',revoked_at=?1,updated_at=?1 WHERE connection_id=?2",now_seconds(),id)?.run().await?;
                    }
                    if let Some(provider)=connection_provider(env,&row.plugin_id) { let _=revoke_provider(&provider,&value).await; }
                }
            }
        }
        worker::query!(&db,"UPDATE account_mcp_oauth_attempts SET status='expired',payload_ciphertext=NULL,verifier='' WHERE attempt_id=?1 AND status='ready' AND expires_at<=?2",&row.attempt_id,now_seconds())?.run().await?;
    }
    worker::query!(&db, "UPDATE account_mcp_oauth_attempts SET status='expired', payload_ciphertext=NULL, verifier='' WHERE expires_at<=?1 AND status NOT IN ('ready','expired','cancelled','consumed')", now_seconds())?.run().await?;
    worker::query!(&db, "DELETE FROM account_mcp_oauth_attempts WHERE expires_at<?1 AND status!='ready'", now_seconds()-86400)?.run().await?;
    Ok(())
}

pub(super) async fn mcp_oauth_start(mut request: Request, context: RouteContext<()>) -> Result<Response> {
    let account = match authenticated_session_account(&request, &context.env).await {
        Ok(a) => a, Err(_) => return error_response(401, "session_required", "请先登录 Fabushi"),
    };
    let body: Start = match request.json().await { Ok(b) => b, Err(_) => return error_response(400,"invalid_connector","未知连接器") };
    if connector(&body.plugin_id).is_none() { return error_response(400,"invalid_connector","未知连接器"); }
    if connection_provider(&context.env, &body.plugin_id).is_none() { return error_response(503,"provider_unconfigured","市场管理员尚未配置服务 OAuth 客户端"); }
    mcp_cleanup(&context.env).await?;
    let db = context.env.d1(ACCOUNT_DATABASE_BINDING)?;
    let attempt_id = Uuid::new_v4().to_string();
    let ticket = format!("{}{}", Uuid::new_v4().simple(), Uuid::new_v4().simple());
    let state = format!("mcp_{}{}", Uuid::new_v4().simple(), Uuid::new_v4().simple());
    let verifier = format!("{}{}", Uuid::new_v4().simple(), Uuid::new_v4().simple());
    // State is derived on authorize from an independent high-entropy ticket;
    // it is never a Fabushi or provider credential.
    let state = format!("mcp_{}", digest(&format!("{state}:{ticket}")));
    let key = context.env.secret("ACCESS_TOKEN_PRIVATE_KEY_PEM")?.to_string();
    let verifier = seal(&key, &attempt_id, format!("{verifier}\n{state}").as_bytes()).map_err(|_| broker_error())?;
    let now = now_seconds();
    worker::query!(&db, "INSERT INTO account_mcp_oauth_attempts (attempt_id,user_id,session_id,plugin_id,state_hash,ticket_hash,verifier,status,created_at,expires_at) VALUES (?1,?2,?3,?4,?5,?6,?7,'pending',?8,?9)", &attempt_id,&account.user_id,account.session_id.as_deref().unwrap_or(""),&body.plugin_id,digest(&state),digest(&ticket),&verifier,now,now+600)?.run().await?;
    let mut url = Url::parse("https://api.ombhrum.com/api/mcp/oauth/authorize").map_err(|_| broker_error())?;
    url.query_pairs_mut().append_pair("attemptId",&attempt_id).append_pair("ticket",&ticket);
    Ok(Response::from_json(&json!({"attemptId":attempt_id,"authorizationUrl":url.as_str(),"expiresAt":(now+600)*1000}))?.with_headers(auth_headers()))
}

pub(super) async fn mcp_oauth_authorize(request: Request, context: RouteContext<()>) -> Result<Response> {
    let url = request.url()?;
    let query = url.query_pairs().collect::<std::collections::HashMap<_,_>>();
    let attempt_id = query.get("attemptId").map(|s| s.as_ref()).unwrap_or("");
    let ticket = query.get("ticket").map(|s| s.as_ref()).unwrap_or("");
    if attempt_id.len()>100 || ticket.len()>160 { return error_response(400,"invalid_attempt","授权链接无效"); }
    let db = context.env.d1(ACCOUNT_DATABASE_BINDING)?;
    let row = worker::query!(&db,"SELECT * FROM account_mcp_oauth_attempts WHERE attempt_id=?1",attempt_id)?.first::<Attempt>(None).await?;
    let Some(row) = row else { return error_response(404,"attempt_missing","授权链接不存在"); };
    if row.status!="pending" || row.expires_at<=now_seconds() || !constant_time_eq(digest(ticket).as_bytes(),row.ticket_hash.as_bytes()) { return error_response(410,"attempt_expired","授权链接已过期"); }
    if ensure_bound_account_session_active(&context.env,&row.user_id,&row.session_id).await.is_err() { return error_response(401,"session_expired","请重新登录 Fabushi"); }
    let Some(provider) = connection_provider(&context.env,&row.plugin_id) else { return error_response(503,"provider_unconfigured","服务 OAuth 尚未配置"); };
    let key = context.env.secret("ACCESS_TOKEN_PRIVATE_KEY_PEM")?.to_string();
    let material = String::from_utf8(open(&key,&row.attempt_id,&row.verifier).map_err(|_| broker_error())?).map_err(|_| broker_error())?;
    let (verifier,state) = material.split_once('\n').ok_or_else(broker_error)?;
    let mut authorize = build_authorization_url(&context.env,&provider,state,callback_url(),verifier).await?;
    if provider.id=="google" { authorize.query_pairs_mut().append_pair("access_type","offline").append_pair("prompt","consent select_account"); }
    let mut response = Response::redirect(authorize)?;
    response.headers_mut().set("Cache-Control","no-store")?;
    response.headers_mut().set("Referrer-Policy","no-referrer")?;
    Ok(response)
}

async fn provider_json(url: &str, method: Method, headers: Headers, body: Option<String>) -> Result<Value> {
    let mut init = RequestInit::new();
    init.with_method(method).with_headers(headers).with_redirect(RequestRedirect::Error)
        .with_body(body.map(|s| wasm_bindgen::JsValue::from_str(&s)));
    let mut response = Fetch::Request(Request::new_with_init(url,&init)?).send().await.map_err(|_| broker_error())?;
    if !(200..300).contains(&response.status_code()) { return Err(broker_error()); }
    let text = response.text().await.map_err(|_| broker_error())?;
    if text.len()>131_072 { return Err(broker_error()); }
    if text.is_empty() { return Ok(json!({})); }
    serde_json::from_str(&text).map_err(|_| broker_error())
}
async fn token_exchange(provider: &OAuthProviderConfig, fields: &[(&str,&str)]) -> Result<Value> {
    let secret = provider.client_secret.as_deref().ok_or_else(broker_error)?;
    let mut form = url::form_urlencoded::Serializer::new(String::new());
    form.append_pair("client_id",&provider.client_id).append_pair("client_secret",secret);
    for (name,value) in fields { form.append_pair(name,value); }
    let headers = Headers::new(); headers.set("Content-Type","application/x-www-form-urlencoded")?; headers.set("Accept","application/json")?;
    let value = provider_json(provider.token_endpoint,Method::Post,headers,Some(form.finish())).await?;
    let token = value.get("access_token").and_then(Value::as_str).filter(|s| !s.is_empty() && s.len()<=32768 && !s.contains('\r') && !s.contains('\n'));
    if token.is_none() || value.get("error").is_some() { return Err(broker_error()); }
    Ok(value)
}
fn credential(tokens: &Value, connection_id: &str, previous_refresh: Option<&str>) -> Value {
    let mut value = json!({"token":tokens["access_token"],"connectionId":connection_id});
    if let Some(refresh) = tokens.get("refresh_token").and_then(Value::as_str).or(previous_refresh) { value["refreshToken"]=json!(refresh); }
    if let Some(expiry) = tokens.get("expires_in").and_then(Value::as_i64).filter(|n| *n>0 && *n<=31536000) { value["expiresAt"]=json!((now_seconds()+expiry)*1000); }
    value
}

async fn complete_attempt(env: &Env, row: &Attempt, code: &str) -> Result<Value> {
    ensure_bound_account_session_active(env,&row.user_id,&row.session_id).await?;
    let provider = connection_provider(env,&row.plugin_id).ok_or_else(broker_error)?;
    let key = env.secret("ACCESS_TOKEN_PRIVATE_KEY_PEM")?.to_string();
    let material = String::from_utf8(open(&key,&row.attempt_id,&row.verifier).map_err(|_| broker_error())?).map_err(|_| broker_error())?;
    let (verifier,_) = material.split_once('\n').ok_or_else(broker_error)?;
    let tokens = token_exchange(&provider,&[("code",code),("redirect_uri",callback_url()),("code_verifier",verifier),("grant_type","authorization_code")]).await?;
    let completed: Result<Value> = async {
    let access = tokens["access_token"].as_str().ok_or_else(broker_error)?;
    let headers = Headers::new(); headers.set("Authorization",&format!("Bearer {access}"))?; headers.set("Accept","application/json")?; headers.set("User-Agent","Fabushi-MCP-Broker")?;
    let profile = provider_json(provider.userinfo_endpoint,Method::Get,headers,None).await?;
    let subject = profile.get("sub").or_else(|| profile.get("id")).map(|s| s.as_str().map(str::to_owned).unwrap_or_else(|| s.to_string())).filter(|s| !s.is_empty() && s.len()<256).ok_or_else(broker_error)?;
    let db = env.d1(ACCOUNT_DATABASE_BINDING)?;
    let now = now_seconds();
    worker::query!(&db,"INSERT OR IGNORE INTO account_principals (principal_id,legacy_user_id,status,created_at,updated_at) VALUES (?1,?2,'active',?3,?3)",format!("prn_legacy_{}",row.user_id),&row.user_id,now)?.run().await?;
    let principal = worker::query!(&db,"SELECT principal_id FROM account_principals WHERE legacy_user_id=?1 AND status='active'",&row.user_id)?.first::<Principal>(None).await?.ok_or_else(broker_error)?;
    let connection_id = Uuid::new_v4().to_string();
    // Use the connector slot as provider to preserve distinct Gmail/Drive grants.
    let scopes = tokens.get("scope").and_then(Value::as_str).unwrap_or("").replace(','," ").split_whitespace().map(str::to_owned).collect::<Vec<_>>();
    #[derive(Deserialize)] struct ConnectionId { connection_id: String }
    let actual = worker::query!(&db,"INSERT INTO account_connections (connection_id,principal_id,provider,provider_subject,display_name,scopes_json,credential_ref,status,created_at,updated_at) VALUES (?1,?2,?3,?4,?5,?6,?7,'active',?8,?8) ON CONFLICT(principal_id,provider,provider_subject) DO UPDATE SET credential_ref=excluded.credential_ref,scopes_json=excluded.scopes_json,status='active',updated_at=excluded.updated_at,revoked_at=NULL RETURNING connection_id",&connection_id,&principal.principal_id,&row.plugin_id,&subject,provider.display_name,serde_json::to_string(&scopes).map_err(|_| broker_error())?,format!("native-mcp:{}",row.attempt_id),now)?.first::<ConnectionId>(None).await?.ok_or_else(broker_error)?;
    let result = credential(&tokens,&actual.connection_id,None);
    worker::query!(&db,"INSERT INTO account_mcp_native_credentials (connection_id,user_id,plugin_id,access_digest,refresh_digest,version) VALUES (?1,?2,?3,?4,?5,1) ON CONFLICT(connection_id) DO UPDATE SET access_digest=excluded.access_digest,refresh_digest=excluded.refresh_digest,version=account_mcp_native_credentials.version+1",&actual.connection_id,&row.user_id,&row.plugin_id,digest(access),result.get("refreshToken").and_then(Value::as_str).map(digest))?.run().await?;
    Ok(result)
    }.await;
    if completed.is_err() { let _ = revoke_provider(&provider,&credential(&tokens,"",None)).await; }
    completed
}

#[event(scheduled)]
pub async fn cleanup_expired_mcp_authorizations(_event: worker::ScheduledEvent, env: Env, _context: worker::ScheduleContext) -> Result<()> {
    mcp_cleanup(&env).await
}

pub(super) async fn mcp_oauth_callback(request: Request, context: RouteContext<()>) -> Result<Response> {
    let url = request.url()?; let query = url.query_pairs().collect::<std::collections::HashMap<_,_>>();
    let state = query.get("state").map(|s| s.as_ref()).unwrap_or("");
    let code = query.get("code").map(|s| s.as_ref()).unwrap_or("");
    if state.len()>160 || code.len()>8192 || state.is_empty() { return error_response(400,"invalid_callback","授权回调无效"); }
    let db = context.env.d1(ACCOUNT_DATABASE_BINDING)?;
    let row = worker::query!(&db,"UPDATE account_mcp_oauth_attempts SET status='exchanging' WHERE state_hash=?1 AND status='pending' AND expires_at>?2 RETURNING *",digest(state),now_seconds())?.first::<Attempt>(None).await?;
    let Some(row) = row else { return error_response(410,"callback_expired","授权回调已处理或过期"); };
    let result = if code.is_empty() || query.contains_key("error") { Err(broker_error()) } else { complete_attempt(&context.env,&row,code).await };
    let success = match result {
        Ok(value) => {
            let key = context.env.secret("ACCESS_TOKEN_PRIVATE_KEY_PEM")?.to_string();
            let ciphertext = seal(&key,&row.attempt_id,value.to_string().as_bytes()).map_err(|_| broker_error())?;
            let committed = worker::query!(&db,"UPDATE account_mcp_oauth_attempts SET status='ready',payload_ciphertext=?1,verifier='' WHERE attempt_id=?2 AND status='exchanging' AND expires_at>?3 RETURNING *",ciphertext,&row.attempt_id,now_seconds())?.first::<Attempt>(None).await?;
            if committed.is_none() {
                // Cancellation raced the exchange: never deliver the resulting grant.
                let connection_id = value["connectionId"].as_str().ok_or_else(broker_error)?;
                worker::query!(&db,"UPDATE account_connections SET status='revoked',revoked_at=?1,updated_at=?1 WHERE connection_id=?2",now_seconds(),connection_id)?.run().await?;
                if let Some(provider) = connection_provider(&context.env,&row.plugin_id) { let _ = revoke_provider(&provider,&value).await; }
                false
            } else { true }
        },
        Err(_) => {
            worker::query!(&db,"UPDATE account_mcp_oauth_attempts SET status='failed',payload_ciphertext=NULL,verifier='' WHERE attempt_id=?1 AND status='exchanging'",&row.attempt_id)?.run().await?;
            false
        },
    };
    let text = if success { "服务授权成功，请返回 Fabushi。" } else { "服务授权未完成，请返回 Fabushi 重新连接。" };
    let mut response = Response::from_html(format!("<!doctype html><meta charset=utf-8><title>Fabushi 服务授权</title><p>{text}</p>"))?;
    response.headers_mut().set("Cache-Control","no-store")?;
    response.headers_mut().set("Referrer-Policy","no-referrer")?;
    response.headers_mut().set("Content-Security-Policy","default-src 'none'; frame-ancestors 'none'; base-uri 'none'")?;
    Ok(response)
}

async fn owned_attempt(request: &Request, context: &RouteContext<()>) -> Result<Option<Attempt>> {
    let account = authenticated_session_account(request,&context.env).await?;
    let db = context.env.d1(ACCOUNT_DATABASE_BINDING)?;
    worker::query!(&db,"SELECT * FROM account_mcp_oauth_attempts WHERE attempt_id=?1 AND user_id=?2 AND session_id=?3",route_identifier(context,"attempt_id")?,&account.user_id,account.session_id.as_deref().unwrap_or(""))?.first::<Attempt>(None).await
}
pub(super) async fn mcp_oauth_poll(request: Request, context: RouteContext<()>) -> Result<Response> {
    let row = match owned_attempt(&request,&context).await { Ok(Some(r))=>r, _=>return error_response(404,"attempt_missing","授权链接不存在") };
    if row.expires_at<=now_seconds() { mcp_cleanup(&context.env).await?; return Ok(Response::from_json(&json!({"status":"expired"}))?.with_headers(auth_headers())); }
    let mut result = json!({"status":row.status});
    if row.status=="ready" {
        let key = context.env.secret("ACCESS_TOKEN_PRIVATE_KEY_PEM")?.to_string();
        let plaintext = open(&key,&row.attempt_id,row.payload_ciphertext.as_deref().ok_or_else(broker_error)?).map_err(|_| broker_error())?;
        result["credential"] = serde_json::from_slice(&plaintext).map_err(|_| broker_error())?;
    }
    Ok(Response::from_json(&result)?.with_headers(auth_headers()))
}
pub(super) async fn mcp_oauth_ack(request: Request, context: RouteContext<()>) -> Result<Response> {
    let row = match owned_attempt(&request,&context).await { Ok(Some(r))=>r, _=>return error_response(404,"attempt_missing","授权链接不存在") };
    let db = context.env.d1(ACCOUNT_DATABASE_BINDING)?;
    worker::query!(&db,"UPDATE account_mcp_oauth_attempts SET status='consumed',payload_ciphertext=NULL,verifier='' WHERE attempt_id=?1 AND status='ready'",&row.attempt_id)?.run().await?;
    Ok(Response::from_json(&json!({"ok":true}))?.with_headers(auth_headers()))
}
pub(super) async fn mcp_oauth_cancel(request: Request, context: RouteContext<()>) -> Result<Response> {
    let row = match owned_attempt(&request,&context).await { Ok(Some(r))=>r, _=>return error_response(404,"attempt_missing","授权链接不存在") };
    let db = context.env.d1(ACCOUNT_DATABASE_BINDING)?;
    worker::query!(&db,"UPDATE account_mcp_oauth_attempts SET status='cancelled',payload_ciphertext=NULL,verifier='' WHERE attempt_id=?1 AND status!='consumed'",&row.attempt_id)?.run().await?;
    if row.status=="ready" {
        let key = context.env.secret("ACCESS_TOKEN_PRIVATE_KEY_PEM")?.to_string();
        if let Some(ciphertext)=row.payload_ciphertext.as_deref() {
            if let Ok(plaintext)=open(&key,&row.attempt_id,ciphertext) {
                if let Ok(value)=serde_json::from_slice::<Value>(&plaintext) {
                    if let Some(id)=value["connectionId"].as_str() {
                        worker::query!(&db,"UPDATE account_connections SET status='revoked',revoked_at=?1,updated_at=?1 WHERE connection_id=?2",now_seconds(),id)?.run().await?;
                    }
                    if let Some(provider)=connection_provider(&context.env,&row.plugin_id) { let _=revoke_provider(&provider,&value).await; }
                }
            }
        }
    }
    Ok(Response::from_json(&json!({"ok":true}))?.with_headers(auth_headers()))
}

async fn owned_connection(request: &Request, context: &RouteContext<()>) -> Result<Option<Connection>> {
    let account=authenticated_session_account(request,&context.env).await?;
    let db=context.env.d1(ACCOUNT_DATABASE_BINDING)?;
    worker::query!(&db,"SELECT n.connection_id,n.plugin_id,n.access_digest,n.refresh_digest,n.version FROM account_mcp_native_credentials n JOIN account_connections c USING(connection_id) WHERE n.connection_id=?1 AND n.user_id=?2 AND c.status='active'",route_identifier(context,"connection_id")?,&account.user_id)?.first::<Connection>(None).await
}
pub(super) async fn mcp_connection_refresh(mut request: Request, context: RouteContext<()>) -> Result<Response> {
    let row=match owned_connection(&request,&context).await { Ok(Some(r))=>r, _=>return error_response(401,"connection_missing","请重新连接服务") };
    let body: Refresh=match request.json().await { Ok(b)=>b, _=>return error_response(400,"invalid_refresh","刷新授权无效") };
    if body.refresh_token.len()>32768 || row.refresh_digest.as_deref()!=Some(digest(&body.refresh_token).as_str()) { return error_response(401,"invalid_refresh","请重新连接服务"); }
    let provider=connection_provider(&context.env,&row.plugin_id).ok_or_else(broker_error)?;
    let tokens=match token_exchange(&provider,&[("grant_type","refresh_token"),("refresh_token",&body.refresh_token)]).await {
        Ok(t)=>t, Err(_)=>return error_response(401,"refresh_failed","授权刷新失败，请重新连接"),
    };
    let value=credential(&tokens,&row.connection_id,Some(&body.refresh_token));
    let db=context.env.d1(ACCOUNT_DATABASE_BINDING)?;
    let updated=worker::query!(&db,"UPDATE account_mcp_native_credentials SET access_digest=?1,refresh_digest=?2,version=version+1 WHERE connection_id=?3 AND version=?4 AND EXISTS(SELECT 1 FROM account_connections c WHERE c.connection_id=?3 AND c.status='active') RETURNING *",digest(value["token"].as_str().ok_or_else(broker_error)?),value["refreshToken"].as_str().map(digest),&row.connection_id,row.version)?.first::<Connection>(None).await?;
    if updated.is_none() { let _=revoke_provider(&provider,&value).await; return error_response(409,"connection_changed","服务连接已改变，请重新连接"); }
    Ok(Response::from_json(&value)?.with_headers(auth_headers()))
}
async fn revoke_provider(provider: &OAuthProviderConfig, credential: &Value) -> Result<()> {
    let token=credential["token"].as_str().ok_or_else(broker_error)?;
    let headers=Headers::new(); headers.set("Accept","application/json")?;
    if provider.id=="google" {
        let refresh=credential["refreshToken"].as_str().unwrap_or(token);
        headers.set("Content-Type","application/x-www-form-urlencoded")?;
        let body=url::form_urlencoded::Serializer::new(String::new()).append_pair("token",refresh).finish();
        provider_json("https://oauth2.googleapis.com/revoke",Method::Post,headers,Some(body)).await?;
    } else {
        let secret=provider.client_secret.as_deref().ok_or_else(broker_error)?;
        let basic=base64::engine::general_purpose::STANDARD.encode(format!("{}:{secret}",provider.client_id));
        headers.set("Authorization",&format!("Basic {basic}"))?; headers.set("Content-Type","application/json")?; headers.set("User-Agent","Fabushi-MCP-Broker")?;
        let url=format!("https://api.github.com/applications/{}/token",provider.client_id);
        provider_json(&url,Method::Delete,headers,Some(json!({"access_token":token}).to_string())).await?;
    }
    Ok(())
}
pub(super) async fn mcp_connection_revoke(mut request: Request, context: RouteContext<()>) -> Result<Response> {
    let row=match owned_connection(&request,&context).await { Ok(Some(r))=>r, _=>return error_response(401,"connection_missing","连接不存在或已撤销") };
    let body: Revoke=match request.json().await { Ok(b)=>b, _=>return error_response(400,"invalid_revoke","撤销授权无效") };
    if body.token.len()>32768 || row.access_digest!=digest(&body.token) || body.refresh_token.as_ref().map(|s| s.len()>32768).unwrap_or(false) || body.refresh_token.as_deref().map(digest)!=row.refresh_digest { return error_response(401,"invalid_revoke","连接凭据已改变"); }
    let db=context.env.d1(ACCOUNT_DATABASE_BINDING)?;
    worker::query!(&db,"UPDATE account_connections SET status='revoked',revoked_at=?1,updated_at=?1 WHERE connection_id=?2",now_seconds(),&row.connection_id)?.run().await?;
    let provider=connection_provider(&context.env,&row.plugin_id).ok_or_else(broker_error)?;
    if revoke_provider(&provider,&json!({"token":body.token,"refreshToken":body.refresh_token})).await.is_err() { return error_response(502,"provider_revoke_failed","本机已断开；请在服务方授权管理页面确认撤销"); }
    Ok(Response::from_json(&json!({"ok":true}))?.with_headers(auth_headers()))
}
