use fabushi_telegram_core::{Command, TelegramEngine};
use fabushi_telegram_network::{establish_auth_key, EstablishedSession, NetworkConfig};
use fabushi_telegram_protocol::{
    build_account_get_password, build_account_send_verify_email_code, build_account_verify_email,
    build_auth_cancel_code, build_auth_check_password, build_auth_check_recovery_password,
    build_auth_request_password_recovery, build_auth_resend_code, build_auth_send_code,
    build_auth_sign_in, build_auth_sign_up, build_init_connection_get_config,
    build_updates_get_state, compute_password_srp_proof, parse_account_email_verified,
    parse_account_password_prefix, parse_account_sent_email_code, parse_auth_password_recovery,
    parse_auth_sent_code, parse_bool, parse_config_dc_directory_prefix, parse_update_state,
    try_parse_rpc_error, AccountPasswordState, AuthCommand, AuthorizationMachine,
    AuthorizationState, CodeDeliveryType, DcDirectory, EmailVerifiedResult, InitConnection,
    PasswordSrpParameters, SentCodeDelivery, SentCodeResult, UpdateState,
};
use fabushi_telegram_storage::{EncryptedSqliteStore, StorageKey};
use once_cell::sync::Lazy;
use serde_json::{json, Value};
use std::{
    collections::BTreeMap,
    sync::{
        atomic::{AtomicU64, Ordering},
        Mutex,
    },
    time::{SystemTime, UNIX_EPOCH},
};
use thiserror::Error;
use zeroize::Zeroizing;

const AUTH_RUNTIME_STATE_KEY: &str = "authorization-runtime-v1";

static NEXT_CLIENT_ID: AtomicU64 = AtomicU64::new(1);
static CLIENTS: Lazy<Mutex<BTreeMap<u64, RuntimeClient>>> =
    Lazy::new(|| Mutex::new(BTreeMap::new()));

struct RuntimeClient {
    core: TelegramEngine,
    authorization: AuthorizationMachine,
    persistence: Option<PersistentClientStorage>,
    transport: Option<EstablishedSession>,
    server_directory: Option<DcDirectory>,
    auth_context: Option<AuthContext>,
    update_state: Option<UpdateState>,
}

struct AuthContext {
    phone_number: Zeroizing<String>,
    phone_code_hash: Zeroizing<String>,
    password_srp: Option<PasswordSrpParameters>,
}

struct PersistentClientStorage {
    store: EncryptedSqliteStore,
    revision: u64,
}

impl Default for RuntimeClient {
    fn default() -> Self {
        Self {
            core: TelegramEngine::new(),
            authorization: AuthorizationMachine::new(),
            persistence: None,
            transport: None,
            server_directory: None,
            auth_context: None,
            update_state: None,
        }
    }
}

#[derive(Debug, Error)]
pub enum RuntimeError {
    #[error("request pointer is null")]
    NullRequestPointer,
    #[error("request is not valid UTF-8")]
    InvalidUtf8,
    #[error("request is not valid JSON: {0}")]
    InvalidJson(#[from] serde_json::Error),
    #[error("request must contain a non-empty @type")]
    MissingRequestType,
    #[error("request type {0} is not supported")]
    UnsupportedRequestType(String),
    #[error("client {0} does not exist")]
    ClientNotFound(u64),
    #[error("runtime client registry is unavailable")]
    RegistryUnavailable,
    #[error("core command failed: {0}")]
    Core(String),
    #[error("authorization command failed: {0}")]
    Authorization(String),
    #[error("encrypted storage failed: {0}")]
    Storage(String),
    #[error("network transport failed: {0}")]
    Network(String),
    #[error("request parameter {0} is invalid")]
    InvalidParameter(&'static str),
    #[error("Telegram transport is not connected")]
    TransportNotConnected,
    #[error("Telegram protocol request failed: {0}")]
    Protocol(String),
    #[error("Telegram authentication code context is unavailable")]
    AuthContextMissing,
    #[error("Telegram RPC failed with {code}: {message}")]
    RemoteRpc { code: i32, message: String },
}

impl RuntimeError {
    pub fn code(&self) -> &'static str {
        match self {
            Self::NullRequestPointer => "null_request_pointer",
            Self::InvalidUtf8 => "invalid_utf8",
            Self::InvalidJson(_) => "invalid_json",
            Self::MissingRequestType => "missing_request_type",
            Self::UnsupportedRequestType(_) => "unsupported_request_type",
            Self::ClientNotFound(_) => "client_not_found",
            Self::RegistryUnavailable => "registry_unavailable",
            Self::Core(_) => "core_command_failed",
            Self::Authorization(_) => "authorization_command_failed",
            Self::Storage(_) => "storage_failed",
            Self::Network(_) => "network_failed",
            Self::InvalidParameter(_) => "invalid_parameter",
            Self::TransportNotConnected => "transport_not_connected",
            Self::Protocol(_) => "protocol_failed",
            Self::AuthContextMissing => "auth_context_missing",
            Self::RemoteRpc { .. } => "telegram_rpc_error",
        }
    }
}

fn auth_context_json(context: &AuthContext) -> Value {
    json!({
        "phoneNumber": context.phone_number.as_str(),
        "phoneCodeHash": context.phone_code_hash.as_str(),
        "passwordSrp": context.password_srp.as_ref().map(|srp| json!({
            "srpId": srp.srp_id,
            "salt1": srp.salt1,
            "salt2": srp.salt2,
            "generator": srp.generator,
            "prime": srp.prime,
            "serverB": srp.server_b,
        })),
    })
}

fn json_bytes(value: &Value, field: &'static str) -> Result<Vec<u8>, RuntimeError> {
    value
        .as_array()
        .ok_or(RuntimeError::InvalidParameter(field))?
        .iter()
        .map(|byte| {
            byte.as_u64()
                .and_then(|value| u8::try_from(value).ok())
                .ok_or(RuntimeError::InvalidParameter(field))
        })
        .collect()
}

fn auth_context_from_json(value: &Value) -> Result<AuthContext, RuntimeError> {
    let phone_number = required_string(value, "phoneNumber")?;
    let phone_code_hash = required_string(value, "phoneCodeHash")?;
    let password_srp = value
        .get("passwordSrp")
        .filter(|value| !value.is_null())
        .map(|srp| {
            Ok::<_, RuntimeError>(PasswordSrpParameters {
                srp_id: srp
                    .get("srpId")
                    .and_then(Value::as_i64)
                    .ok_or(RuntimeError::InvalidParameter("srpId"))?,
                salt1: json_bytes(
                    srp.get("salt1")
                        .ok_or(RuntimeError::InvalidParameter("salt1"))?,
                    "salt1",
                )?,
                salt2: json_bytes(
                    srp.get("salt2")
                        .ok_or(RuntimeError::InvalidParameter("salt2"))?,
                    "salt2",
                )?,
                generator: srp
                    .get("generator")
                    .and_then(Value::as_i64)
                    .and_then(|value| i32::try_from(value).ok())
                    .ok_or(RuntimeError::InvalidParameter("generator"))?,
                prime: json_bytes(
                    srp.get("prime")
                        .ok_or(RuntimeError::InvalidParameter("prime"))?,
                    "prime",
                )?,
                server_b: json_bytes(
                    srp.get("serverB")
                        .ok_or(RuntimeError::InvalidParameter("serverB"))?,
                    "serverB",
                )?,
            })
        })
        .transpose()?;
    Ok(AuthContext {
        phone_number: Zeroizing::new(phone_number.to_string()),
        phone_code_hash: Zeroizing::new(phone_code_hash.to_string()),
        password_srp,
    })
}

fn serialize_auth_runtime_state(client: &RuntimeClient) -> Result<Vec<u8>, RuntimeError> {
    serde_json::to_vec(&json!({
        "authorizationState": client.authorization.state(),
        "authContext": client.auth_context.as_ref().map(auth_context_json),
    }))
    .map_err(RuntimeError::from)
}

fn restore_auth_runtime_state(
    bytes: Option<Vec<u8>>,
) -> Result<(AuthorizationMachine, Option<AuthContext>), RuntimeError> {
    let Some(bytes) = bytes else {
        return Ok((AuthorizationMachine::new(), None));
    };
    let value: Value = serde_json::from_slice(&bytes)?;
    let state: AuthorizationState = serde_json::from_value(
        value
            .get("authorizationState")
            .cloned()
            .ok_or(RuntimeError::InvalidParameter("authorizationState"))?,
    )?;
    let context = value
        .get("authContext")
        .filter(|value| !value.is_null())
        .map(auth_context_from_json)
        .transpose()?;
    Ok((AuthorizationMachine::from_state(state), context))
}

fn persist_auth_runtime_state(client: &mut RuntimeClient) -> Result<(), RuntimeError> {
    let payload = serialize_auth_runtime_state(client)?;
    if let Some(persistence) = &mut client.persistence {
        persistence
            .store
            .save_secure_value(AUTH_RUNTIME_STATE_KEY, &payload)
            .map_err(|error| RuntimeError::Storage(error.to_string()))?;
    }
    Ok(())
}

pub fn create_client() -> u64 {
    insert_client(RuntimeClient::default())
}

pub fn create_persistent_client(
    database_path: &str,
    storage_key: &[u8],
) -> Result<u64, RuntimeError> {
    let key = StorageKey::from_slice(storage_key)
        .map_err(|error| RuntimeError::Storage(error.to_string()))?;
    let store = EncryptedSqliteStore::open(database_path, key)
        .map_err(|error| RuntimeError::Storage(error.to_string()))?;
    let snapshot = store
        .load_snapshot()
        .map_err(|error| RuntimeError::Storage(error.to_string()))?;
    let auth_runtime_state = store
        .load_secure_value(AUTH_RUNTIME_STATE_KEY)
        .map_err(|error| RuntimeError::Storage(error.to_string()))?;
    let (authorization, auth_context) = restore_auth_runtime_state(auth_runtime_state)?;
    let (core, revision) = snapshot.map_or_else(
        || (TelegramEngine::new(), 0),
        |snapshot| {
            (
                TelegramEngine::from_state(snapshot.state),
                snapshot.revision,
            )
        },
    );
    Ok(insert_client(RuntimeClient {
        core,
        authorization,
        persistence: Some(PersistentClientStorage { store, revision }),
        transport: None,
        server_directory: None,
        auth_context,
        update_state: None,
    }))
}

fn insert_client(client: RuntimeClient) -> u64 {
    let client_id = NEXT_CLIENT_ID.fetch_add(1, Ordering::Relaxed);
    if let Ok(mut clients) = CLIENTS.lock() {
        clients.insert(client_id, client);
    }
    client_id
}

pub fn close_client(client_id: u64) -> Result<(), RuntimeError> {
    let mut clients = CLIENTS
        .lock()
        .map_err(|_| RuntimeError::RegistryUnavailable)?;
    clients
        .remove(&client_id)
        .map(|_| ())
        .ok_or(RuntimeError::ClientNotFound(client_id))
}

pub fn execute_json(client_id: u64, request_json: &str) -> String {
    let request = serde_json::from_str::<Value>(request_json);
    let extra = request
        .as_ref()
        .ok()
        .and_then(|value| value.get("@extra").cloned());
    let result = request
        .map_err(RuntimeError::from)
        .and_then(|request| execute_value(client_id, request));
    match result {
        Ok(mut data) => {
            if let Some(extra) = extra {
                if let Some(object) = data.as_object_mut() {
                    object.insert("@extra".to_string(), extra);
                }
            }
            json!({"ok": true, "data": data}).to_string()
        }
        Err(error) => json!({
            "ok": false,
            "errorCode": error.code(),
            "message": error.to_string(),
            "@extra": extra,
        })
        .to_string(),
    }
}

fn execute_value(client_id: u64, request: Value) -> Result<Value, RuntimeError> {
    let request_type = request
        .get("@type")
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .ok_or(RuntimeError::MissingRequestType)?;
    if request_type == "telegram.bootstrapTransport" {
        return bootstrap_transport(client_id, &request);
    }
    if request_type == "telegram.initializeConnection" {
        return initialize_connection(client_id, &request);
    }
    if request_type == "telegram.sendAuthenticationCode" {
        return send_authentication_code(client_id, &request);
    }
    if request_type == "telegram.submitAuthenticationCode" {
        return submit_authentication_code(client_id, &request);
    }
    if request_type == "telegram.submitAuthenticationPassword" {
        return submit_authentication_password(client_id, &request);
    }
    if request_type == "telegram.submitRegistration" {
        return submit_registration(client_id, &request);
    }
    if request_type == "telegram.resendAuthenticationCode" {
        return resend_authentication_code(client_id, &request);
    }
    if request_type == "telegram.cancelAuthenticationCode" {
        return cancel_authentication_code(client_id);
    }
    if request_type == "telegram.requestPasswordRecovery" {
        return request_password_recovery(client_id);
    }
    if request_type == "telegram.checkRecoveryPassword" {
        return check_recovery_password(client_id, &request);
    }
    if request_type == "telegram.sendVerifyEmailCode" {
        return send_verify_email_code(client_id, &request);
    }
    if request_type == "telegram.verifyEmail" {
        return verify_email(client_id, &request);
    }
    if request_type == "telegram.refreshPasswordChallenge" {
        return refresh_password_challenge(client_id);
    }
    if request_type == "telegram.beginUpdateSync" {
        return begin_update_sync(client_id);
    }
    let mut clients = CLIENTS
        .lock()
        .map_err(|_| RuntimeError::RegistryUnavailable)?;
    let client = clients
        .get_mut(&client_id)
        .ok_or(RuntimeError::ClientNotFound(client_id))?;

    match request_type {
        "telegram.getStatus" => {
            let transport = client.transport.as_ref();
            Ok(json!({
                "@type": "telegram.status",
                "clientId": client_id,
                "architecture": "rust-command-event-core",
                "persistentStorage": client.persistence.is_some(),
                "transportConnected": transport.is_some(),
                "dcId": transport.map(|session| session.endpoint.dc_id),
                "endpoint": transport.map(|session| session.endpoint.address.to_string()),
                "authKeyId": transport.map(|session| format!("{:016x}", session.auth_key.id())),
                "serverTime": transport.map(|session| session.server_time),
                "serverDcOptions": client.server_directory.as_ref().map(|value| value.endpoints().len()),
                "updateState": client.update_state,
            }))
        }
        "telegram.getState" => Ok(json!({
            "@type": "telegram.state",
            "state": client.core.state(),
        })),
        "telegram.getAuthorizationState" => Ok(json!({
            "@type": "telegram.authorizationState",
            "authorizationState": client.authorization.state(),
        })),
        "telegram.executeCoreCommand" => {
            let command: Command = serde_json::from_value(
                request
                    .get("command")
                    .cloned()
                    .ok_or(RuntimeError::MissingRequestType)?,
            )?;
            let events = client
                .core
                .decide(command)
                .map_err(|error| RuntimeError::Core(error.to_string()))?;
            let mut next_core = client.core.clone();
            for event in &events {
                next_core.apply(event.clone());
            }
            if let Some(persistence) = &mut client.persistence {
                let transition = persistence
                    .store
                    .commit_transition(
                        next_core.state(),
                        &events,
                        persistence.revision,
                        now_unix_ms(),
                    )
                    .map_err(|error| RuntimeError::Storage(error.to_string()))?;
                persistence.revision = transition.snapshot.revision;
            }
            client.core = next_core;
            Ok(json!({
                "@type": "telegram.coreResult",
                "events": events,
                "state": client.core.state(),
            }))
        }
        "telegram.executeAuthorizationCommand" => {
            let command: AuthCommand = serde_json::from_value(
                request
                    .get("command")
                    .cloned()
                    .ok_or(RuntimeError::MissingRequestType)?,
            )?;
            let events = client
                .authorization
                .execute(command)
                .map_err(|error| RuntimeError::Authorization(error.to_string()))?;
            persist_auth_runtime_state(client)?;
            Ok(json!({
                "@type": "telegram.authorizationResult",
                "events": events,
                "authorizationState": client.authorization.state(),
            }))
        }
        other => Err(RuntimeError::UnsupportedRequestType(other.to_string())),
    }
}

fn bootstrap_transport(client_id: u64, request: &Value) -> Result<Value, RuntimeError> {
    {
        let clients = CLIENTS
            .lock()
            .map_err(|_| RuntimeError::RegistryUnavailable)?;
        if !clients.contains_key(&client_id) {
            return Err(RuntimeError::ClientNotFound(client_id));
        }
    }
    let dc_id = request.get("dcId").and_then(Value::as_i64).unwrap_or(2);
    let dc_id = i32::try_from(dc_id)
        .ok()
        .filter(|value| *value > 0)
        .ok_or(RuntimeError::InvalidParameter("dcId"))?;
    let test_mode = request
        .get("testMode")
        .map(Value::as_bool)
        .unwrap_or(Some(false))
        .ok_or(RuntimeError::InvalidParameter("testMode"))?;
    let directory = fabushi_telegram_protocol::DcDirectory::telegram_defaults(test_mode, dc_id)
        .map_err(|error| RuntimeError::Network(error.to_string()))?;
    let mut session = establish_auth_key(&directory, dc_id, &NetworkConfig::default())
        .map_err(|error| RuntimeError::Network(error.to_string()))?;
    let pong = session
        .ping()
        .map_err(|error| RuntimeError::Network(error.to_string()))?;
    let response = json!({
        "@type": "telegram.transportReady",
        "dcId": session.endpoint.dc_id,
        "endpoint": session.endpoint.address.to_string(),
        "authKeyId": format!("{:016x}", session.auth_key.id()),
        "serverTime": session.server_time,
        "encryptedPingVerified": true,
        "pongMessageId": pong.response_message_id.to_string(),
    });
    let mut clients = CLIENTS
        .lock()
        .map_err(|_| RuntimeError::RegistryUnavailable)?;
    let client = clients
        .get_mut(&client_id)
        .ok_or(RuntimeError::ClientNotFound(client_id))?;
    client.transport = Some(session);
    client.server_directory = None;
    Ok(response)
}

fn initialize_connection(client_id: u64, request: &Value) -> Result<Value, RuntimeError> {
    let api_id = request
        .get("apiId")
        .and_then(Value::as_i64)
        .and_then(|value| i32::try_from(value).ok())
        .filter(|value| *value > 0)
        .ok_or(RuntimeError::InvalidParameter("apiId"))?;
    let device_model = required_string(request, "deviceModel")?;
    let system_version = required_string(request, "systemVersion")?;
    let app_version = required_string(request, "appVersion")?;
    let system_lang_code = required_string(request, "systemLangCode")?;
    let lang_pack = request
        .get("langPack")
        .and_then(Value::as_str)
        .unwrap_or("");
    let lang_code = required_string(request, "langCode")?;
    let body = build_init_connection_get_config(&InitConnection {
        api_id,
        device_model,
        system_version,
        app_version,
        system_lang_code,
        lang_pack,
        lang_code,
    })
    .map_err(|error| RuntimeError::Protocol(error.to_string()))?;

    let mut session = {
        let mut clients = CLIENTS
            .lock()
            .map_err(|_| RuntimeError::RegistryUnavailable)?;
        let client = clients
            .get_mut(&client_id)
            .ok_or(RuntimeError::ClientNotFound(client_id))?;
        client
            .transport
            .take()
            .ok_or(RuntimeError::TransportNotConnected)?
    };
    let operation = session
        .invoke_raw(&body)
        .map_err(|error| RuntimeError::Network(error.to_string()))
        .and_then(|result| {
            parse_config_dc_directory_prefix(&result.body)
                .map_err(|error| RuntimeError::Protocol(error.to_string()))
        });

    let mut clients = CLIENTS
        .lock()
        .map_err(|_| RuntimeError::RegistryUnavailable)?;
    let client = clients
        .get_mut(&client_id)
        .ok_or(RuntimeError::ClientNotFound(client_id))?;
    client.transport = Some(session);
    let config = operation?;
    let response = json!({
        "@type": "telegram.connectionInitialized",
        "apiLayer": fabushi_telegram_protocol::MTPROTO_LAYER,
        "thisDc": config.directory.this_dc(),
        "testMode": config.directory.test_mode(),
        "dcOptionCount": config.directory.endpoints().len(),
        "configDate": config.date,
        "configExpires": config.expires,
    });
    client.server_directory = Some(config.directory);
    Ok(response)
}

fn required_string<'a>(request: &'a Value, field: &'static str) -> Result<&'a str, RuntimeError> {
    request
        .get(field)
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .ok_or(RuntimeError::InvalidParameter(field))
}

fn send_authentication_code(client_id: u64, request: &Value) -> Result<Value, RuntimeError> {
    let phone_number = required_string(request, "phoneNumber")?;
    let api_id = request
        .get("apiId")
        .and_then(Value::as_i64)
        .and_then(|value| i32::try_from(value).ok())
        .filter(|value| *value > 0)
        .ok_or(RuntimeError::InvalidParameter("apiId"))?;
    let api_hash = required_string(request, "apiHash")?;
    let body = build_auth_send_code(phone_number, api_id, api_hash)
        .map_err(|error| RuntimeError::Protocol(error.to_string()))?;

    let mut session = take_transport(client_id)?;
    let operation = session
        .invoke_raw(&body)
        .map_err(|error| RuntimeError::Network(error.to_string()))
        .and_then(|result| {
            parse_auth_sent_code(&result.body)
                .map_err(|error| RuntimeError::Protocol(error.to_string()))
        });
    let mut clients = CLIENTS
        .lock()
        .map_err(|_| RuntimeError::RegistryUnavailable)?;
    let client = clients
        .get_mut(&client_id)
        .ok_or(RuntimeError::ClientNotFound(client_id))?;
    client.transport = Some(session);
    let result = operation?;

    match result {
        SentCodeResult::Code { code } => {
            let (delivery_type, code_length) = delivery_state(&code.delivery);
            let timeout_seconds = code
                .timeout_seconds
                .and_then(|value| u32::try_from(value).ok());
            let state = AuthorizationState::WaitCode {
                phone_number: phone_number.to_string(),
                delivery_type,
                code_length,
                timeout_seconds,
            };
            let events = client
                .authorization
                .execute(AuthCommand::ApplyRemoteState {
                    state: state.clone(),
                })
                .map_err(|error| RuntimeError::Authorization(error.to_string()))?;
            client.auth_context = Some(AuthContext {
                phone_number: Zeroizing::new(phone_number.to_string()),
                phone_code_hash: Zeroizing::new(code.phone_code_hash),
                password_srp: None,
            });
            persist_auth_runtime_state(client)?;
            Ok(json!({
                "@type": "telegram.authenticationCodeSent",
                "events": events,
                "authorizationState": state,
                "delivery": code.delivery,
                "nextType": code.next_type,
            }))
        }
        SentCodeResult::Success => {
            let state = AuthorizationState::Ready;
            let events = client
                .authorization
                .execute(AuthCommand::ApplyRemoteState {
                    state: state.clone(),
                })
                .map_err(|error| RuntimeError::Authorization(error.to_string()))?;
            client.auth_context = None;
            persist_auth_runtime_state(client)?;
            Ok(json!({
                "@type": "telegram.authorizationReady",
                "events": events,
                "authorizationState": state,
            }))
        }
        payment @ SentCodeResult::PaymentRequired { .. } => Ok(json!({
            "@type": "telegram.authenticationPaymentRequired",
            "details": payment,
            "authorizationState": client.authorization.state(),
        })),
    }
}

fn take_transport(client_id: u64) -> Result<EstablishedSession, RuntimeError> {
    let mut clients = CLIENTS
        .lock()
        .map_err(|_| RuntimeError::RegistryUnavailable)?;
    let client = clients
        .get_mut(&client_id)
        .ok_or(RuntimeError::ClientNotFound(client_id))?;
    client
        .transport
        .take()
        .ok_or(RuntimeError::TransportNotConnected)
}

fn delivery_state(delivery: &SentCodeDelivery) -> (CodeDeliveryType, u8) {
    let (delivery_type, length) = match delivery {
        SentCodeDelivery::App { length } => (CodeDeliveryType::TelegramMessage, *length),
        SentCodeDelivery::Sms { length } => (CodeDeliveryType::Sms, *length),
        SentCodeDelivery::SmsWord { .. } | SentCodeDelivery::SmsPhrase { .. } => {
            (CodeDeliveryType::Sms, 0)
        }
        SentCodeDelivery::Call { length } => (CodeDeliveryType::Call, *length),
        SentCodeDelivery::FlashCall { .. } => (CodeDeliveryType::FlashCall, 0),
        SentCodeDelivery::MissedCall { length, .. } => (CodeDeliveryType::MissedCall, *length),
        SentCodeDelivery::Email { length, .. } => (CodeDeliveryType::Email, *length),
        SentCodeDelivery::SetUpEmailRequired => (CodeDeliveryType::Email, 0),
        SentCodeDelivery::FragmentSms { length, .. } => (CodeDeliveryType::Fragment, *length),
        SentCodeDelivery::FirebaseSms { length } => (CodeDeliveryType::FirebaseAndroid, *length),
    };
    (delivery_type, u8::try_from(length).unwrap_or(0))
}

fn submit_authentication_code(client_id: u64, request: &Value) -> Result<Value, RuntimeError> {
    let phone_code = required_string(request, "code")?;
    let (phone_number, phone_code_hash) = {
        let clients = CLIENTS
            .lock()
            .map_err(|_| RuntimeError::RegistryUnavailable)?;
        let client = clients
            .get(&client_id)
            .ok_or(RuntimeError::ClientNotFound(client_id))?;
        let context = client
            .auth_context
            .as_ref()
            .ok_or(RuntimeError::AuthContextMissing)?;
        (
            Zeroizing::new(context.phone_number.to_string()),
            Zeroizing::new(context.phone_code_hash.to_string()),
        )
    };
    let body = build_auth_sign_in(&phone_number, &phone_code_hash, phone_code)
        .map_err(|error| RuntimeError::Protocol(error.to_string()))?;
    let mut session = take_transport(client_id)?;
    let operation = (|| -> Result<SubmitCodeOutcome, RuntimeError> {
        let result = session
            .invoke_raw(&body)
            .map_err(|error| RuntimeError::Network(error.to_string()))?;
        if let Some(error) = try_parse_rpc_error(&result.body)
            .map_err(|error| RuntimeError::Protocol(error.to_string()))?
        {
            if error.message == "SESSION_PASSWORD_NEEDED" {
                let password = session
                    .invoke_raw(&build_account_get_password())
                    .map_err(|error| RuntimeError::Network(error.to_string()))?;
                Ok(SubmitCodeOutcome::Password(
                    parse_account_password_prefix(&password.body)
                        .map_err(|error| RuntimeError::Protocol(error.to_string()))?,
                ))
            } else {
                Ok(SubmitCodeOutcome::RemoteError {
                    code: error.code,
                    message: error.message,
                })
            }
        } else {
            let constructor = result
                .body
                .get(..4)
                .map(|bytes| u32::from_le_bytes(bytes.try_into().expect("four-byte slice")))
                .ok_or_else(|| {
                    RuntimeError::Protocol("authorization response is truncated".to_string())
                })?;
            match constructor {
                0x2ea2_c0d4 => Ok(SubmitCodeOutcome::Ready),
                0x4474_7e9a => Ok(SubmitCodeOutcome::Registration),
                _ => Err(RuntimeError::Protocol(format!(
                    "unexpected authorization constructor 0x{constructor:08x}"
                ))),
            }
        }
    })();
    let mut clients = CLIENTS
        .lock()
        .map_err(|_| RuntimeError::RegistryUnavailable)?;
    let client = clients
        .get_mut(&client_id)
        .ok_or(RuntimeError::ClientNotFound(client_id))?;
    client.transport = Some(session);
    let outcome = operation?;
    let (response_type, state) = match outcome {
        SubmitCodeOutcome::Password(password) => {
            let srp = password.srp.ok_or_else(|| {
                RuntimeError::Protocol(
                    "account.Password omitted current SRP parameters".to_string(),
                )
            })?;
            let state = AuthorizationState::WaitPassword {
                password_hint: password.hint.unwrap_or_default(),
                has_recovery_email: password.has_recovery,
                recovery_email_pattern: password.email_unconfirmed_pattern,
            };
            let context = client
                .auth_context
                .as_mut()
                .ok_or(RuntimeError::AuthContextMissing)?;
            context.password_srp = Some(srp);
            ("telegram.authenticationPasswordRequired", state)
        }
        SubmitCodeOutcome::Ready => ("telegram.authorizationReady", AuthorizationState::Ready),
        SubmitCodeOutcome::Registration => (
            "telegram.authenticationRegistrationRequired",
            AuthorizationState::WaitRegistration {
                terms_of_service_id: None,
            },
        ),
        SubmitCodeOutcome::RemoteError { code, message } => {
            return Err(RuntimeError::RemoteRpc { code, message });
        }
    };
    let events = client
        .authorization
        .execute(AuthCommand::ApplyRemoteState {
            state: state.clone(),
        })
        .map_err(|error| RuntimeError::Authorization(error.to_string()))?;
    if matches!(state, AuthorizationState::Ready) {
        client.auth_context = None;
    }
    persist_auth_runtime_state(client)?;
    Ok(json!({
        "@type": response_type,
        "events": events,
        "authorizationState": state,
    }))
}

enum SubmitCodeOutcome {
    Ready,
    Registration,
    Password(AccountPasswordState),
    RemoteError { code: i32, message: String },
}

fn submit_authentication_password(client_id: u64, request: &Value) -> Result<Value, RuntimeError> {
    let password = Zeroizing::new(required_string(request, "password")?.to_string());
    let parameters = {
        let clients = CLIENTS
            .lock()
            .map_err(|_| RuntimeError::RegistryUnavailable)?;
        let client = clients
            .get(&client_id)
            .ok_or(RuntimeError::ClientNotFound(client_id))?;
        client
            .auth_context
            .as_ref()
            .and_then(|context| context.password_srp.clone())
            .ok_or(RuntimeError::AuthContextMissing)?
    };
    let proof = compute_password_srp_proof(&password, &parameters)
        .map_err(|error| RuntimeError::Protocol(error.to_string()))?;
    let body = build_auth_check_password(&proof)
        .map_err(|error| RuntimeError::Protocol(error.to_string()))?;
    let mut session = take_transport(client_id)?;
    let operation = (|| -> Result<(), RuntimeError> {
        let result = session
            .invoke_raw(&body)
            .map_err(|error| RuntimeError::Network(error.to_string()))?;
        if let Some(error) = try_parse_rpc_error(&result.body)
            .map_err(|error| RuntimeError::Protocol(error.to_string()))?
        {
            return Err(RuntimeError::RemoteRpc {
                code: error.code,
                message: error.message,
            });
        }
        let constructor = result
            .body
            .get(..4)
            .map(|bytes| u32::from_le_bytes(bytes.try_into().expect("four-byte slice")))
            .ok_or_else(|| {
                RuntimeError::Protocol("authorization response is truncated".to_string())
            })?;
        if constructor != 0x2ea2_c0d4 {
            Err(RuntimeError::Protocol(format!(
                "unexpected authorization constructor 0x{constructor:08x}"
            )))
        } else {
            Ok(())
        }
    })();

    let mut clients = CLIENTS
        .lock()
        .map_err(|_| RuntimeError::RegistryUnavailable)?;
    let client = clients
        .get_mut(&client_id)
        .ok_or(RuntimeError::ClientNotFound(client_id))?;
    client.transport = Some(session);
    operation?;
    let state = AuthorizationState::Ready;
    let events = client
        .authorization
        .execute(AuthCommand::ApplyRemoteState {
            state: state.clone(),
        })
        .map_err(|error| RuntimeError::Authorization(error.to_string()))?;
    client.auth_context = None;
    persist_auth_runtime_state(client)?;
    Ok(json!({
        "@type": "telegram.authorizationReady",
        "events": events,
        "authorizationState": state,
    }))
}

fn submit_registration(client_id: u64, request: &Value) -> Result<Value, RuntimeError> {
    let first_name = required_string(request, "firstName")?;
    let last_name = request
        .get("lastName")
        .and_then(Value::as_str)
        .unwrap_or("");
    let (phone_number, phone_code_hash) = {
        let clients = CLIENTS
            .lock()
            .map_err(|_| RuntimeError::RegistryUnavailable)?;
        let client = clients
            .get(&client_id)
            .ok_or(RuntimeError::ClientNotFound(client_id))?;
        let context = client
            .auth_context
            .as_ref()
            .ok_or(RuntimeError::AuthContextMissing)?;
        (
            Zeroizing::new(context.phone_number.to_string()),
            Zeroizing::new(context.phone_code_hash.to_string()),
        )
    };
    let body = build_auth_sign_up(&phone_number, &phone_code_hash, first_name, last_name)
        .map_err(|error| RuntimeError::Protocol(error.to_string()))?;
    let mut session = take_transport(client_id)?;
    let operation = (|| -> Result<(), RuntimeError> {
        let result = session
            .invoke_raw(&body)
            .map_err(|error| RuntimeError::Network(error.to_string()))?;
        if let Some(error) = try_parse_rpc_error(&result.body)
            .map_err(|error| RuntimeError::Protocol(error.to_string()))?
        {
            return Err(RuntimeError::RemoteRpc {
                code: error.code,
                message: error.message,
            });
        }
        let constructor = result
            .body
            .get(..4)
            .map(|bytes| u32::from_le_bytes(bytes.try_into().expect("four-byte slice")))
            .ok_or_else(|| {
                RuntimeError::Protocol("authorization response is truncated".to_string())
            })?;
        if constructor == 0x2ea2_c0d4 {
            Ok(())
        } else {
            Err(RuntimeError::Protocol(format!(
                "unexpected authorization constructor 0x{constructor:08x}"
            )))
        }
    })();
    let mut clients = CLIENTS
        .lock()
        .map_err(|_| RuntimeError::RegistryUnavailable)?;
    let client = clients
        .get_mut(&client_id)
        .ok_or(RuntimeError::ClientNotFound(client_id))?;
    client.transport = Some(session);
    operation?;
    let state = AuthorizationState::Ready;
    let events = client
        .authorization
        .execute(AuthCommand::ApplyRemoteState {
            state: state.clone(),
        })
        .map_err(|error| RuntimeError::Authorization(error.to_string()))?;
    client.auth_context = None;
    Ok(json!({
        "@type": "telegram.authorizationReady",
        "events": events,
        "authorizationState": state,
    }))
}


fn auth_context_phone_hash(
    client_id: u64,
) -> Result<(Zeroizing<String>, Zeroizing<String>), RuntimeError> {
    let clients = CLIENTS
        .lock()
        .map_err(|_| RuntimeError::RegistryUnavailable)?;
    let client = clients
        .get(&client_id)
        .ok_or(RuntimeError::ClientNotFound(client_id))?;
    let context = client
        .auth_context
        .as_ref()
        .ok_or(RuntimeError::AuthContextMissing)?;
    Ok((
        Zeroizing::new(context.phone_number.to_string()),
        Zeroizing::new(context.phone_code_hash.to_string()),
    ))
}

fn rpc_error_from_body(body: &[u8]) -> Result<Option<RuntimeError>, RuntimeError> {
    Ok(try_parse_rpc_error(body)
        .map_err(|error| RuntimeError::Protocol(error.to_string()))?
        .map(|error| RuntimeError::RemoteRpc {
            code: error.code,
            message: error.message,
        }))
}

fn resend_authentication_code(client_id: u64, request: &Value) -> Result<Value, RuntimeError> {
    let reason = request.get("reason").and_then(Value::as_str);
    let (phone_number, phone_code_hash) = auth_context_phone_hash(client_id)?;
    let body = build_auth_resend_code(&phone_number, &phone_code_hash, reason)
        .map_err(|error| RuntimeError::Protocol(error.to_string()))?;
    let mut session = take_transport(client_id)?;
    let operation = (|| -> Result<SentCodeResult, RuntimeError> {
        let result = session
            .invoke_raw(&body)
            .map_err(|error| RuntimeError::Network(error.to_string()))?;
        if let Some(error) = rpc_error_from_body(&result.body)? {
            return Err(error);
        }
        parse_auth_sent_code(&result.body)
            .map_err(|error| RuntimeError::Protocol(error.to_string()))
    })();
    let mut clients = CLIENTS
        .lock()
        .map_err(|_| RuntimeError::RegistryUnavailable)?;
    let client = clients
        .get_mut(&client_id)
        .ok_or(RuntimeError::ClientNotFound(client_id))?;
    client.transport = Some(session);
    match operation? {
        SentCodeResult::Code { code } => {
            let (delivery_type, code_length) = delivery_state(&code.delivery);
            let state = AuthorizationState::WaitCode {
                phone_number: phone_number.to_string(),
                delivery_type,
                code_length,
                timeout_seconds: code
                    .timeout_seconds
                    .and_then(|value| u32::try_from(value).ok()),
            };
            let events = client
                .authorization
                .execute(AuthCommand::ApplyRemoteState {
                    state: state.clone(),
                })
                .map_err(|error| RuntimeError::Authorization(error.to_string()))?;
            client.auth_context = Some(AuthContext {
                phone_number,
                phone_code_hash: Zeroizing::new(code.phone_code_hash),
                password_srp: None,
            });
            persist_auth_runtime_state(client)?;
            Ok(json!({
                "@type": "telegram.authenticationCodeResent",
                "events": events,
                "authorizationState": state,
                "delivery": code.delivery,
                "nextType": code.next_type,
            }))
        }
        SentCodeResult::Success => {
            let state = AuthorizationState::Ready;
            let events = client
                .authorization
                .execute(AuthCommand::ApplyRemoteState {
                    state: state.clone(),
                })
                .map_err(|error| RuntimeError::Authorization(error.to_string()))?;
            client.auth_context = None;
            persist_auth_runtime_state(client)?;
            Ok(json!({
                "@type": "telegram.authorizationReady",
                "events": events,
                "authorizationState": state,
            }))
        }
        payment @ SentCodeResult::PaymentRequired { .. } => Ok(json!({
            "@type": "telegram.authenticationPaymentRequired",
            "details": payment,
            "authorizationState": client.authorization.state(),
        })),
    }
}

fn cancel_authentication_code(client_id: u64) -> Result<Value, RuntimeError> {
    let (phone_number, phone_code_hash) = auth_context_phone_hash(client_id)?;
    let body = build_auth_cancel_code(&phone_number, &phone_code_hash)
        .map_err(|error| RuntimeError::Protocol(error.to_string()))?;
    let mut session = take_transport(client_id)?;
    let operation = (|| -> Result<(), RuntimeError> {
        let result = session
            .invoke_raw(&body)
            .map_err(|error| RuntimeError::Network(error.to_string()))?;
        if let Some(error) = rpc_error_from_body(&result.body)? {
            return Err(error);
        }
        if !parse_bool(&result.body)
            .map_err(|error| RuntimeError::Protocol(error.to_string()))?
        {
            return Err(RuntimeError::Protocol(
                "auth.cancelCode returned Bool false".to_string(),
            ));
        }
        Ok(())
    })();
    let mut clients = CLIENTS
        .lock()
        .map_err(|_| RuntimeError::RegistryUnavailable)?;
    let client = clients
        .get_mut(&client_id)
        .ok_or(RuntimeError::ClientNotFound(client_id))?;
    client.transport = Some(session);
    operation?;
    let state = AuthorizationState::WaitPhoneNumber;
    let events = client
        .authorization
        .execute(AuthCommand::ApplyRemoteState {
            state: state.clone(),
        })
        .map_err(|error| RuntimeError::Authorization(error.to_string()))?;
    client.auth_context = None;
    persist_auth_runtime_state(client)?;
    Ok(json!({
        "@type": "telegram.authenticationCodeCancelled",
        "events": events,
        "authorizationState": state,
    }))
}

fn request_password_recovery(client_id: u64) -> Result<Value, RuntimeError> {
    {
        let clients = CLIENTS
            .lock()
            .map_err(|_| RuntimeError::RegistryUnavailable)?;
        let client = clients
            .get(&client_id)
            .ok_or(RuntimeError::ClientNotFound(client_id))?;
        if !matches!(client.authorization.state(), AuthorizationState::WaitPassword { .. }) {
            return Err(RuntimeError::Authorization(
                "password recovery requires waitPassword state".to_string(),
            ));
        }
    }
    let mut session = take_transport(client_id)?;
    let operation = (|| {
        let result = session
            .invoke_raw(&build_auth_request_password_recovery())
            .map_err(|error| RuntimeError::Network(error.to_string()))?;
        if let Some(error) = rpc_error_from_body(&result.body)? {
            return Err(error);
        }
        parse_auth_password_recovery(&result.body)
            .map_err(|error| RuntimeError::Protocol(error.to_string()))
    })();
    let mut clients = CLIENTS
        .lock()
        .map_err(|_| RuntimeError::RegistryUnavailable)?;
    let client = clients
        .get_mut(&client_id)
        .ok_or(RuntimeError::ClientNotFound(client_id))?;
    client.transport = Some(session);
    let recovery = operation?;
    let events = client
        .authorization
        .execute(AuthCommand::RequestPasswordRecovery)
        .map_err(|error| RuntimeError::Authorization(error.to_string()))?;
    Ok(json!({
        "@type": "telegram.passwordRecoveryRequested",
        "events": events,
        "emailPattern": recovery.email_pattern,
        "authorizationState": client.authorization.state(),
    }))
}

fn check_recovery_password(client_id: u64, request: &Value) -> Result<Value, RuntimeError> {
    let code = required_string(request, "code")?;
    {
        let clients = CLIENTS
            .lock()
            .map_err(|_| RuntimeError::RegistryUnavailable)?;
        let client = clients
            .get(&client_id)
            .ok_or(RuntimeError::ClientNotFound(client_id))?;
        if !matches!(client.authorization.state(), AuthorizationState::WaitPassword { .. }) {
            return Err(RuntimeError::Authorization(
                "recovery code validation requires waitPassword state".to_string(),
            ));
        }
    }
    let body = build_auth_check_recovery_password(code)
        .map_err(|error| RuntimeError::Protocol(error.to_string()))?;
    let mut session = take_transport(client_id)?;
    let operation = (|| -> Result<(), RuntimeError> {
        let result = session
            .invoke_raw(&body)
            .map_err(|error| RuntimeError::Network(error.to_string()))?;
        if let Some(error) = rpc_error_from_body(&result.body)? {
            return Err(error);
        }
        if !parse_bool(&result.body)
            .map_err(|error| RuntimeError::Protocol(error.to_string()))?
        {
            return Err(RuntimeError::Protocol(
                "auth.checkRecoveryPassword returned Bool false".to_string(),
            ));
        }
        Ok(())
    })();
    let mut clients = CLIENTS
        .lock()
        .map_err(|_| RuntimeError::RegistryUnavailable)?;
    let client = clients
        .get_mut(&client_id)
        .ok_or(RuntimeError::ClientNotFound(client_id))?;
    client.transport = Some(session);
    operation?;
    Ok(json!({
        "@type": "telegram.passwordRecoveryCodeAccepted",
        "authorizationState": client.authorization.state(),
    }))
}

fn send_verify_email_code(client_id: u64, request: &Value) -> Result<Value, RuntimeError> {
    let email = required_string(request, "email")?;
    let (phone_number, phone_code_hash) = auth_context_phone_hash(client_id)?;
    let body = build_account_send_verify_email_code(&phone_number, &phone_code_hash, email)
        .map_err(|error| RuntimeError::Protocol(error.to_string()))?;
    let mut session = take_transport(client_id)?;
    let operation = (|| {
        let result = session
            .invoke_raw(&body)
            .map_err(|error| RuntimeError::Network(error.to_string()))?;
        if let Some(error) = rpc_error_from_body(&result.body)? {
            return Err(error);
        }
        parse_account_sent_email_code(&result.body)
            .map_err(|error| RuntimeError::Protocol(error.to_string()))
    })();
    let mut clients = CLIENTS
        .lock()
        .map_err(|_| RuntimeError::RegistryUnavailable)?;
    let client = clients
        .get_mut(&client_id)
        .ok_or(RuntimeError::ClientNotFound(client_id))?;
    client.transport = Some(session);
    let sent = operation?;
    Ok(json!({
        "@type": "telegram.authenticationEmailCodeSent",
        "emailPattern": sent.email_pattern,
        "codeLength": sent.length,
        "authorizationState": client.authorization.state(),
    }))
}

fn verify_email(client_id: u64, request: &Value) -> Result<Value, RuntimeError> {
    let code = required_string(request, "code")?;
    let (phone_number, phone_code_hash) = auth_context_phone_hash(client_id)?;
    let body = build_account_verify_email(&phone_number, &phone_code_hash, code)
        .map_err(|error| RuntimeError::Protocol(error.to_string()))?;
    let mut session = take_transport(client_id)?;
    let operation = (|| {
        let result = session
            .invoke_raw(&body)
            .map_err(|error| RuntimeError::Network(error.to_string()))?;
        if let Some(error) = rpc_error_from_body(&result.body)? {
            return Err(error);
        }
        parse_account_email_verified(&result.body)
            .map_err(|error| RuntimeError::Protocol(error.to_string()))
    })();
    let mut clients = CLIENTS
        .lock()
        .map_err(|_| RuntimeError::RegistryUnavailable)?;
    let client = clients
        .get_mut(&client_id)
        .ok_or(RuntimeError::ClientNotFound(client_id))?;
    client.transport = Some(session);
    match operation? {
        EmailVerifiedResult::LoginCode { email, code } => {
            let (delivery_type, code_length) = delivery_state(&code.delivery);
            let state = AuthorizationState::WaitCode {
                phone_number: phone_number.to_string(),
                delivery_type,
                code_length,
                timeout_seconds: code
                    .timeout_seconds
                    .and_then(|value| u32::try_from(value).ok()),
            };
            let events = client
                .authorization
                .execute(AuthCommand::ApplyRemoteState {
                    state: state.clone(),
                })
                .map_err(|error| RuntimeError::Authorization(error.to_string()))?;
            client.auth_context = Some(AuthContext {
                phone_number,
                phone_code_hash: Zeroizing::new(code.phone_code_hash),
                password_srp: None,
            });
            persist_auth_runtime_state(client)?;
            Ok(json!({
                "@type": "telegram.authenticationEmailVerified",
                "email": email,
                "events": events,
                "authorizationState": state,
                "delivery": code.delivery,
                "nextType": code.next_type,
            }))
        }
        EmailVerifiedResult::LoginSuccess { email } => {
            let state = AuthorizationState::Ready;
            let events = client
                .authorization
                .execute(AuthCommand::ApplyRemoteState {
                    state: state.clone(),
                })
                .map_err(|error| RuntimeError::Authorization(error.to_string()))?;
            client.auth_context = None;
            persist_auth_runtime_state(client)?;
            Ok(json!({
                "@type": "telegram.authorizationReady",
                "email": email,
                "events": events,
                "authorizationState": state,
            }))
        }
        EmailVerifiedResult::LoginPaymentRequired { .. } => Err(RuntimeError::Protocol(
            "email verification returned authentication payment requirement".to_string(),
        )),
        EmailVerifiedResult::Verified { .. } => Err(RuntimeError::Protocol(
            "login email verification returned non-login account.emailVerified".to_string(),
        )),
    }
}

fn refresh_password_challenge(client_id: u64) -> Result<Value, RuntimeError> {
    {
        let clients = CLIENTS
            .lock()
            .map_err(|_| RuntimeError::RegistryUnavailable)?;
        let client = clients
            .get(&client_id)
            .ok_or(RuntimeError::ClientNotFound(client_id))?;
        if !matches!(client.authorization.state(), AuthorizationState::WaitPassword { .. }) {
            return Err(RuntimeError::Authorization(
                "password challenge refresh requires waitPassword state".to_string(),
            ));
        }
        if client.auth_context.is_none() {
            return Err(RuntimeError::AuthContextMissing);
        }
    }
    let mut session = take_transport(client_id)?;
    let operation = (|| -> Result<AccountPasswordState, RuntimeError> {
        let result = session
            .invoke_raw(&build_account_get_password())
            .map_err(|error| RuntimeError::Network(error.to_string()))?;
        if let Some(error) = rpc_error_from_body(&result.body)? {
            return Err(error);
        }
        parse_account_password_prefix(&result.body)
            .map_err(|error| RuntimeError::Protocol(error.to_string()))
    })();
    let mut clients = CLIENTS
        .lock()
        .map_err(|_| RuntimeError::RegistryUnavailable)?;
    let client = clients
        .get_mut(&client_id)
        .ok_or(RuntimeError::ClientNotFound(client_id))?;
    client.transport = Some(session);
    let password = operation?;
    let srp = password.srp.ok_or_else(|| {
        RuntimeError::Protocol("account.Password omitted current SRP parameters".to_string())
    })?;
    let state = AuthorizationState::WaitPassword {
        password_hint: password.hint.unwrap_or_default(),
        has_recovery_email: password.has_recovery,
        recovery_email_pattern: password.email_unconfirmed_pattern,
    };
    let events = client
        .authorization
        .execute(AuthCommand::ApplyRemoteState {
            state: state.clone(),
        })
        .map_err(|error| RuntimeError::Authorization(error.to_string()))?;
    client
        .auth_context
        .as_mut()
        .ok_or(RuntimeError::AuthContextMissing)?
        .password_srp = Some(srp);
    persist_auth_runtime_state(client)?;
    Ok(json!({
        "@type": "telegram.authenticationPasswordChallengeRefreshed",
        "events": events,
        "authorizationState": state,
    }))
}

fn begin_update_sync(client_id: u64) -> Result<Value, RuntimeError> {
    {
        let clients = CLIENTS
            .lock()
            .map_err(|_| RuntimeError::RegistryUnavailable)?;
        let client = clients
            .get(&client_id)
            .ok_or(RuntimeError::ClientNotFound(client_id))?;
        if !matches!(client.authorization.state(), AuthorizationState::Ready) {
            return Err(RuntimeError::Authorization(
                "Telegram account must be authorized before update sync".to_string(),
            ));
        }
    }
    let mut session = take_transport(client_id)?;
    let operation = session
        .invoke_raw(&build_updates_get_state())
        .map_err(|error| RuntimeError::Network(error.to_string()))
        .and_then(|result| {
            parse_update_state(&result.body)
                .map_err(|error| RuntimeError::Protocol(error.to_string()))
        });
    let mut clients = CLIENTS
        .lock()
        .map_err(|_| RuntimeError::RegistryUnavailable)?;
    let client = clients
        .get_mut(&client_id)
        .ok_or(RuntimeError::ClientNotFound(client_id))?;
    client.transport = Some(session);
    let state = operation?;
    client.update_state = Some(state);
    Ok(json!({
        "@type": "telegram.updateSyncStarted",
        "state": state,
    }))
}

fn now_unix_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .ok()
        .and_then(|duration| i64::try_from(duration.as_millis()).ok())
        .unwrap_or(0)
}
