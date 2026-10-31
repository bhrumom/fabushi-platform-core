use fabushi_telegram_core::Command;
use fabushi_telegram_core::TelegramEngine;
use fabushi_telegram_network::establish_auth_key;
use fabushi_telegram_network::resume_auth_key;
use fabushi_telegram_network::EstablishedSession;
use fabushi_telegram_network::NetworkConfig;
use fabushi_telegram_protocol::build_account_get_password;
use fabushi_telegram_protocol::build_account_send_verify_email_code;
use fabushi_telegram_protocol::build_account_verify_email;
use fabushi_telegram_protocol::build_auth_cancel_code;
use fabushi_telegram_protocol::build_auth_check_password;
use fabushi_telegram_protocol::build_auth_check_recovery_password;
use fabushi_telegram_protocol::build_auth_request_password_recovery;
use fabushi_telegram_protocol::build_auth_resend_code;
use fabushi_telegram_protocol::build_auth_send_code;
use fabushi_telegram_protocol::build_auth_sign_in;
use fabushi_telegram_protocol::build_auth_sign_up;
use fabushi_telegram_protocol::build_init_connection_get_config;
use fabushi_telegram_protocol::build_updates_get_state;
use fabushi_telegram_protocol::compute_password_srp_proof;
use fabushi_telegram_protocol::parse_account_email_verified;
use fabushi_telegram_protocol::parse_account_password_prefix;
use fabushi_telegram_protocol::parse_account_sent_email_code;
use fabushi_telegram_protocol::parse_auth_password_recovery;
use fabushi_telegram_protocol::parse_auth_sent_code;
use fabushi_telegram_protocol::parse_bool;
use fabushi_telegram_protocol::parse_auth_authorization_user_id;
use fabushi_telegram_protocol::parse_config_dc_directory_prefix;
use fabushi_telegram_protocol::parse_update_state;
use fabushi_telegram_protocol::try_parse_rpc_error;
use fabushi_telegram_protocol::AccountPasswordState;
use fabushi_telegram_protocol::AuthCommand;
use fabushi_telegram_protocol::AuthEvent;
use fabushi_telegram_protocol::AuthKey;
use fabushi_telegram_protocol::AuthorizationMachine;
use fabushi_telegram_protocol::AuthorizationState;
use fabushi_telegram_protocol::CodeDeliveryType;
use fabushi_telegram_protocol::DcDirectory;
use fabushi_telegram_protocol::EmailVerifiedResult;
use fabushi_telegram_protocol::InitConnection;
use fabushi_telegram_protocol::PasswordSrpParameters;
use fabushi_telegram_protocol::SentCodeDelivery;
use fabushi_telegram_protocol::SentCodeResult;
use fabushi_telegram_protocol::UpdateState;
use fabushi_telegram_protocol::AUTH_AUTHORIZATION_CONSTRUCTOR;
use fabushi_telegram_protocol::AUTH_AUTHORIZATION_SIGN_UP_REQUIRED_CONSTRUCTOR;
use fabushi_telegram_storage::EncryptedSqliteStore;
use fabushi_telegram_storage::StorageKey;
use once_cell::sync::Lazy;
use serde::Deserialize;
use serde::Serialize;
use serde_json::json;
use serde_json::Value;
use std::collections::BTreeMap;
use std::sync::atomic::AtomicU64;
use std::sync::atomic::Ordering;
use std::sync::Mutex;
use std::time::SystemTime;
use std::time::UNIX_EPOCH;
use thiserror::Error;
use zeroize::Zeroize;
use zeroize::Zeroizing;

static NEXT_CLIENT_ID: AtomicU64 = AtomicU64::new(1);
static CLIENTS: Lazy<Mutex<BTreeMap<u64, RuntimeClient>>> =
    Lazy::new(|| Mutex::new(BTreeMap::new()));

const AUTHORIZATION_METADATA_KEY: &str = "authorization.v1";
const TRANSPORT_METADATA_KEY: &str = "transport.v1";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuthorizationSnapshot {
    pub state: AuthorizationState,
    pub self_user_id: Option<i64>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PersistedAuthorization {
    authorization_state: AuthorizationState,
    self_user_id: Option<i64>,
    auth_context: Option<PersistedAuthContext>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PersistedAuthContext {
    phone_number: String,
    phone_code_hash: String,
    password_srp: Option<PasswordSrpParameters>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PersistedTransportState {
    dc_id: i32,
    test_mode: bool,
    server_salt: i64,
    server_time: i32,
    auth_key: Vec<u8>,
}

impl Drop for PersistedTransportState {
    fn drop(&mut self) {
        self.auth_key.zeroize();
    }
}

struct RuntimeClient {
    core: TelegramEngine,
    authorization: AuthorizationMachine,
    self_user_id: Option<i64>,
    persistence: Option<PersistentClientStorage>,
    transport: Option<EstablishedSession>,
    persisted_transport: Option<PersistedTransportState>,
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
            self_user_id: None,
            persistence: None,
            transport: None,
            persisted_transport: None,
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
    let authorization_payload = store
        .load_private_metadata(AUTHORIZATION_METADATA_KEY)
        .map_err(|error| RuntimeError::Storage(error.to_string()))?
        .map(Zeroizing::new);
    let transport_payload = store
        .load_private_metadata(TRANSPORT_METADATA_KEY)
        .map_err(|error| RuntimeError::Storage(error.to_string()))?
        .map(Zeroizing::new);
    let (core, revision) = snapshot.map_or_else(
        || (TelegramEngine::new(), 0),
        |snapshot| {
            (
                TelegramEngine::from_state(snapshot.state),
                snapshot.revision,
            )
        },
    );
    let persisted_authorization = authorization_payload
        .as_ref()
        .map(|payload| serde_json::from_slice::<PersistedAuthorization>(payload.as_slice()))
        .transpose()
        .map_err(|error| RuntimeError::Storage(format!(
            "decode Telegram authorization metadata: {error}"
        )))?;
    let persisted_transport = transport_payload
        .as_ref()
        .map(|payload| serde_json::from_slice::<PersistedTransportState>(payload.as_slice()))
        .transpose()
        .map_err(|error| RuntimeError::Storage(format!(
            "decode Telegram transport metadata: {error}"
        )))?;
    let (authorization_state, self_user_id, auth_context) =
        if let Some(persisted) = persisted_authorization {
            let auth_context = persisted.auth_context.map(|context| AuthContext {
                phone_number: Zeroizing::new(context.phone_number),
                phone_code_hash: Zeroizing::new(context.phone_code_hash),
                password_srp: context.password_srp,
            });
            (
                persisted.authorization_state,
                persisted.self_user_id,
                auth_context,
            )
        } else {
            (AuthorizationState::WaitParameters, None, None)
        };
    validate_restored_authorization(
        &authorization_state,
        self_user_id,
        auth_context.as_ref(),
        persisted_transport.as_ref(),
    )?;
    Ok(insert_client(RuntimeClient {
        core,
        authorization: AuthorizationMachine::from_state(authorization_state),
        self_user_id,
        persistence: Some(PersistentClientStorage { store, revision }),
        transport: None,
        persisted_transport,
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

pub fn authorization_snapshot(client_id: u64) -> Result<AuthorizationSnapshot, RuntimeError> {
    let clients = CLIENTS
        .lock()
        .map_err(|_| RuntimeError::RegistryUnavailable)?;
    let client = clients
        .get(&client_id)
        .ok_or(RuntimeError::ClientNotFound(client_id))?;
    Ok(AuthorizationSnapshot {
        state: client.authorization.state().clone(),
        self_user_id: client.self_user_id,
    })
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
                "transportRestorable": client.persisted_transport.is_some(),
                "authorizationState": client.authorization.state(),
                "selfUserId": client.self_user_id,
                "dcId": transport
                    .map(|session| session.endpoint.dc_id)
                    .or_else(|| client.persisted_transport.as_ref().map(|state| state.dc_id)),
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
            "selfUserId": client.self_user_id,
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
            if matches!(
                &command,
                AuthCommand::ApplyRemoteState {
                    state: AuthorizationState::Ready
                }
            ) {
                return Err(RuntimeError::Authorization(
                    "Ready authorization requires a canonical Telegram self user identity".to_string(),
                ));
            }
            let events = client
                .authorization
                .execute(command)
                .map_err(|error| RuntimeError::Authorization(error.to_string()))?;
            if !matches!(client.authorization.state(), AuthorizationState::Ready) {
                client.self_user_id = None;
            }
            persist_authorization(client)?;
            Ok(json!({
                "@type": "telegram.authorizationResult",
                "events": events,
                "authorizationState": client.authorization.state(),
                "selfUserId": client.self_user_id,
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
    if !matches!(
        client.authorization.state(),
        AuthorizationState::WaitParameters | AuthorizationState::WaitPhoneNumber
    ) {
        client.auth_context = None;
        apply_authorization_state(client, AuthorizationState::WaitParameters, None)?;
    }
    put_transport(client, session, Some(test_mode))?;
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

    let mut session = take_transport(client_id)?;
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
    put_transport(client, session, None)?;
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
    put_transport(client, session, None)?;
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
            client.auth_context = Some(AuthContext {
                phone_number: Zeroizing::new(phone_number.to_string()),
                phone_code_hash: Zeroizing::new(code.phone_code_hash),
                password_srp: None,
            });
            let events = apply_authorization_state(client, state.clone(), None)?;
            Ok(json!({
                "@type": "telegram.authenticationCodeSent",
                "events": events,
                "authorizationState": state,
                "selfUserId": client.self_user_id,
                "delivery": code.delivery,
                "nextType": code.next_type,
            }))
        }
        SentCodeResult::Success { user_id } => {
            let state = AuthorizationState::Ready;
            client.auth_context = None;
            let events = apply_authorization_state(client, state.clone(), Some(user_id))?;
            Ok(json!({
                "@type": "telegram.authorizationReady",
                "events": events,
                "authorizationState": state,
                "selfUserId": user_id,
            }))
        }
        payment @ SentCodeResult::PaymentRequired { .. } => Ok(json!({
            "@type": "telegram.authenticationPaymentRequired",
            "details": payment,
            "authorizationState": client.authorization.state(),
            "selfUserId": client.self_user_id,
        })),
    }
}

fn take_transport(client_id: u64) -> Result<EstablishedSession, RuntimeError> {
    let resume = {
        let mut clients = CLIENTS
            .lock()
            .map_err(|_| RuntimeError::RegistryUnavailable)?;
        let client = clients
            .get_mut(&client_id)
            .ok_or(RuntimeError::ClientNotFound(client_id))?;
        if let Some(session) = client.transport.take() {
            return Ok(session);
        }
        client.persisted_transport.clone()
    };
    let resume = resume.ok_or(RuntimeError::TransportNotConnected)?;
    let directory = DcDirectory::telegram_defaults(resume.test_mode, resume.dc_id)
        .map_err(|error| RuntimeError::Network(error.to_string()))?;
    let auth_key = AuthKey::from_slice(&resume.auth_key)
        .map_err(|error| RuntimeError::Protocol(error.to_string()))?;
    resume_auth_key(
        &directory,
        resume.dc_id,
        auth_key,
        resume.server_salt,
        resume.server_time,
        &NetworkConfig::default(),
    )
    .map_err(|error| RuntimeError::Network(error.to_string()))
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
                .ok_or_else(|| RuntimeError::Protocol("authorization response is truncated".to_string()))?;
            match constructor {
                AUTH_AUTHORIZATION_CONSTRUCTOR => Ok(SubmitCodeOutcome::Ready(
                    parse_auth_authorization_user_id(&result.body)
                        .map_err(|error| RuntimeError::Protocol(error.to_string()))?,
                )),
                AUTH_AUTHORIZATION_SIGN_UP_REQUIRED_CONSTRUCTOR => Ok(SubmitCodeOutcome::Registration),
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
    put_transport(client, session, None)?;
    let outcome = operation?;
    let (response_type, state, self_user_id) = match outcome {
        SubmitCodeOutcome::Password(password) => {
            let srp = password.srp.ok_or_else(|| {
                RuntimeError::Protocol("account.Password omitted current SRP parameters".to_string())
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
            ("telegram.authenticationPasswordRequired", state, None)
        }
        SubmitCodeOutcome::Ready(user_id) => {
            client.auth_context = None;
            ("telegram.authorizationReady", AuthorizationState::Ready, Some(user_id))
        }
        SubmitCodeOutcome::Registration => (
            "telegram.authenticationRegistrationRequired",
            AuthorizationState::WaitRegistration {
                terms_of_service_id: None,
            },
            None,
        ),
        SubmitCodeOutcome::RemoteError { code, message } => {
            return Err(RuntimeError::RemoteRpc { code, message });
        }
    };
    let events = apply_authorization_state(client, state.clone(), self_user_id)?;
    Ok(json!({
        "@type": response_type,
        "events": events,
        "authorizationState": state,
        "selfUserId": client.self_user_id,
    }))
}

enum SubmitCodeOutcome {
    Ready(i64),
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
    let operation = (|| -> Result<i64, RuntimeError> {
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
        parse_auth_authorization_user_id(&result.body)
            .map_err(|error| RuntimeError::Protocol(error.to_string()))
    })();

    let mut clients = CLIENTS
        .lock()
        .map_err(|_| RuntimeError::RegistryUnavailable)?;
    let client = clients
        .get_mut(&client_id)
        .ok_or(RuntimeError::ClientNotFound(client_id))?;
    put_transport(client, session, None)?;
    let user_id = operation?;
    client.auth_context = None;
    let state = AuthorizationState::Ready;
    let events = apply_authorization_state(client, state.clone(), Some(user_id))?;
    Ok(json!({
        "@type": "telegram.authorizationReady",
        "events": events,
        "authorizationState": state,
        "selfUserId": user_id,
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
    let operation = (|| -> Result<i64, RuntimeError> {
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
        parse_auth_authorization_user_id(&result.body)
            .map_err(|error| RuntimeError::Protocol(error.to_string()))
    })();
    let mut clients = CLIENTS
        .lock()
        .map_err(|_| RuntimeError::RegistryUnavailable)?;
    let client = clients
        .get_mut(&client_id)
        .ok_or(RuntimeError::ClientNotFound(client_id))?;
    put_transport(client, session, None)?;
    let user_id = operation?;
    client.auth_context = None;
    let state = AuthorizationState::Ready;
    let events = apply_authorization_state(client, state.clone(), Some(user_id))?;
    Ok(json!({
        "@type": "telegram.authorizationReady",
        "events": events,
        "authorizationState": state,
        "selfUserId": user_id,
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
    put_transport(client, session, None)?;
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
            client.auth_context = Some(AuthContext {
                phone_number,
                phone_code_hash: Zeroizing::new(code.phone_code_hash),
                password_srp: None,
            });
            let events = apply_authorization_state(client, state.clone(), None)?;
            Ok(json!({
                "@type": "telegram.authenticationCodeResent",
                "events": events,
                "authorizationState": state,
                "selfUserId": client.self_user_id,
                "delivery": code.delivery,
                "nextType": code.next_type,
            }))
        }
        SentCodeResult::Success { user_id } => {
            let state = AuthorizationState::Ready;
            client.auth_context = None;
            let events = apply_authorization_state(client, state.clone(), Some(user_id))?;
            Ok(json!({
                "@type": "telegram.authorizationReady",
                "events": events,
                "authorizationState": state,
                "selfUserId": client.self_user_id,
            }))
        }
        payment @ SentCodeResult::PaymentRequired { .. } => Ok(json!({
            "@type": "telegram.authenticationPaymentRequired",
            "details": payment,
            "authorizationState": client.authorization.state(),
        "selfUserId": client.self_user_id,
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
    put_transport(client, session, None)?;
    operation?;
    let state = AuthorizationState::WaitPhoneNumber;
    client.auth_context = None;
    let events = apply_authorization_state(client, state.clone(), None)?;
    Ok(json!({
        "@type": "telegram.authenticationCodeCancelled",
        "events": events,
        "authorizationState": state,
                "selfUserId": client.self_user_id,
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
    put_transport(client, session, None)?;
    let recovery = operation?;
    let events = client
        .authorization
        .execute(AuthCommand::RequestPasswordRecovery)
        .map_err(|error| RuntimeError::Authorization(error.to_string()))?;
    persist_authorization(client)?;
    Ok(json!({
        "@type": "telegram.passwordRecoveryRequested",
        "events": events,
        "emailPattern": recovery.email_pattern,
        "authorizationState": client.authorization.state(),
        "selfUserId": client.self_user_id,
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
    put_transport(client, session, None)?;
    operation?;
    Ok(json!({
        "@type": "telegram.passwordRecoveryCodeAccepted",
        "authorizationState": client.authorization.state(),
        "selfUserId": client.self_user_id,
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
    put_transport(client, session, None)?;
    let sent = operation?;
    Ok(json!({
        "@type": "telegram.authenticationEmailCodeSent",
        "emailPattern": sent.email_pattern,
        "codeLength": sent.length,
        "authorizationState": client.authorization.state(),
        "selfUserId": client.self_user_id,
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
    put_transport(client, session, None)?;
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
            client.auth_context = Some(AuthContext {
                phone_number,
                phone_code_hash: Zeroizing::new(code.phone_code_hash),
                password_srp: None,
            });
            let events = apply_authorization_state(client, state.clone(), None)?;
            Ok(json!({
                "@type": "telegram.authenticationEmailVerified",
                "email": email,
                "events": events,
                "authorizationState": state,
                "selfUserId": client.self_user_id,
                "delivery": code.delivery,
                "nextType": code.next_type,
            }))
        }
        EmailVerifiedResult::LoginSuccess { email, user_id } => {
            let state = AuthorizationState::Ready;
            client.auth_context = None;
            let events = apply_authorization_state(client, state.clone(), Some(user_id))?;
            Ok(json!({
                "@type": "telegram.authorizationReady",
                "email": email,
                "events": events,
                "authorizationState": state,
                "selfUserId": client.self_user_id,
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
    put_transport(client, session, None)?;
    let password = operation?;
    let srp = password.srp.ok_or_else(|| {
        RuntimeError::Protocol("account.Password omitted current SRP parameters".to_string())
    })?;
    let state = AuthorizationState::WaitPassword {
        password_hint: password.hint.unwrap_or_default(),
        has_recovery_email: password.has_recovery,
        recovery_email_pattern: password.email_unconfirmed_pattern,
    };
    client
        .auth_context
        .as_mut()
        .ok_or(RuntimeError::AuthContextMissing)?
        .password_srp = Some(srp);
    let events = apply_authorization_state(client, state.clone(), None)?;
    Ok(json!({
        "@type": "telegram.authenticationPasswordChallengeRefreshed",
        "events": events,
        "authorizationState": state,
                "selfUserId": client.self_user_id,
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
        if !matches!(client.authorization.state(), AuthorizationState::Ready)
            || client.self_user_id.is_none()
        {
            return Err(RuntimeError::Authorization(
                "Telegram account must be authorized with a canonical self user identity before update sync"
                    .to_string(),
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
    put_transport(client, session, None)?;
    let state = operation?;
    client.update_state = Some(state);
    Ok(json!({
        "@type": "telegram.updateSyncStarted",
        "state": state,
        "selfUserId": client.self_user_id,
    }))
}

fn validate_restored_authorization(
    state: &AuthorizationState,
    self_user_id: Option<i64>,
    auth_context: Option<&AuthContext>,
    transport: Option<&PersistedTransportState>,
) -> Result<(), RuntimeError> {
    if matches!(state, AuthorizationState::Ready) {
        if self_user_id.is_none() || transport.is_none() {
            return Err(RuntimeError::Storage(
                "persisted Ready authorization is missing self-user identity or MTProto auth-key material"
                    .to_string(),
            ));
        }
    } else if self_user_id.is_some() {
        return Err(RuntimeError::Storage(
            "persisted Telegram self-user identity is only valid for Ready authorization".to_string(),
        ));
    }
    if matches!(
        state,
        AuthorizationState::WaitCode { .. }
            | AuthorizationState::WaitPassword { .. }
            | AuthorizationState::WaitRegistration { .. }
    ) && (auth_context.is_none() || transport.is_none())
    {
        return Err(RuntimeError::Storage(
            "persisted Telegram authorization challenge is missing resumable context".to_string(),
        ));
    }
    Ok(())
}

fn persist_authorization(client: &mut RuntimeClient) -> Result<(), RuntimeError> {
    let encoded = {
        let auth_context = client.auth_context.as_ref().map(|context| PersistedAuthContext {
            phone_number: context.phone_number.to_string(),
            phone_code_hash: context.phone_code_hash.to_string(),
            password_srp: context.password_srp.clone(),
        });
        let persisted = PersistedAuthorization {
            authorization_state: client.authorization.state().clone(),
            self_user_id: client.self_user_id,
            auth_context,
        };
        Zeroizing::new(
            serde_json::to_vec(&persisted)
                .map_err(|error| RuntimeError::Storage(error.to_string()))?,
        )
    };
    if let Some(persistence) = &mut client.persistence {
        persistence
            .store
            .save_private_metadata(
                AUTHORIZATION_METADATA_KEY,
                encoded.as_slice(),
                now_unix_ms(),
            )
            .map_err(|error| RuntimeError::Storage(error.to_string()))?;
    }
    Ok(())
}

fn persist_transport_state(client: &mut RuntimeClient) -> Result<(), RuntimeError> {
    let Some(state) = client.persisted_transport.as_ref() else {
        return Ok(());
    };
    let encoded = Zeroizing::new(
        serde_json::to_vec(state).map_err(|error| RuntimeError::Storage(error.to_string()))?,
    );
    if let Some(persistence) = &mut client.persistence {
        persistence
            .store
            .save_private_metadata(
                TRANSPORT_METADATA_KEY,
                encoded.as_slice(),
                now_unix_ms(),
            )
            .map_err(|error| RuntimeError::Storage(error.to_string()))?;
    }
    Ok(())
}

fn put_transport(
    client: &mut RuntimeClient,
    session: EstablishedSession,
    test_mode: Option<bool>,
) -> Result<(), RuntimeError> {
    let test_mode = test_mode
        .or_else(|| client.persisted_transport.as_ref().map(|state| state.test_mode))
        .unwrap_or(false);
    client.persisted_transport = Some(PersistedTransportState {
        dc_id: session.endpoint.dc_id,
        test_mode,
        server_salt: session.server_salt,
        server_time: session.server_time,
        auth_key: session.auth_key.as_bytes().to_vec(),
    });
    persist_transport_state(client)?;
    client.transport = Some(session);
    Ok(())
}

fn apply_authorization_state(
    client: &mut RuntimeClient,
    state: AuthorizationState,
    self_user_id: Option<i64>,
) -> Result<Vec<AuthEvent>, RuntimeError> {
    if matches!(state, AuthorizationState::Ready) {
        if self_user_id.is_none() || client.persisted_transport.is_none() {
            return Err(RuntimeError::Authorization(
                "Ready authorization requires canonical self-user identity and MTProto auth-key material"
                    .to_string(),
            ));
        }
    } else if self_user_id.is_some() {
        return Err(RuntimeError::Authorization(
            "Telegram self-user identity is only valid for Ready authorization".to_string(),
        ));
    }
    let events = client
        .authorization
        .execute(AuthCommand::ApplyRemoteState {
            state: state.clone(),
        })
        .map_err(|error| RuntimeError::Authorization(error.to_string()))?;
    client.self_user_id = self_user_id;
    persist_authorization(client)?;
    Ok(events)
}

fn now_unix_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .ok()
        .and_then(|duration| i64::try_from(duration.as_millis()).ok())
        .unwrap_or(0)
}


#[cfg(test)]
mod authorization_restore_contract_tests {
    use super::*;

    #[test]
    fn persistent_ready_authorization_restores_identity_without_opening_second_runtime() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("telegram.sqlite3");
        let key_bytes = [11_u8; 32];
        let mut store = EncryptedSqliteStore::open(
            &path,
            StorageKey::from_slice(&key_bytes).unwrap(),
        )
        .unwrap();
        let authorization = PersistedAuthorization {
            authorization_state: AuthorizationState::Ready,
            self_user_id: Some(4_242),
            auth_context: None,
        };
        store
            .save_private_metadata(
                AUTHORIZATION_METADATA_KEY,
                &serde_json::to_vec(&authorization).unwrap(),
                1,
            )
            .unwrap();
        let transport = PersistedTransportState {
            dc_id: 2,
            test_mode: false,
            server_salt: 7,
            server_time: 8,
            auth_key: vec![3_u8; 256],
        };
        store
            .save_private_metadata(
                TRANSPORT_METADATA_KEY,
                &serde_json::to_vec(&transport).unwrap(),
                1,
            )
            .unwrap();
        drop(store);

        let client_id =
            create_persistent_client(path.to_str().unwrap(), &key_bytes).unwrap();
        assert_eq!(
            authorization_snapshot(client_id).unwrap(),
            AuthorizationSnapshot {
                state: AuthorizationState::Ready,
                self_user_id: Some(4_242),
            }
        );
        close_client(client_id).unwrap();
    }

    #[test]
    fn persistent_ready_authorization_fails_closed_without_auth_key_material() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("telegram.sqlite3");
        let key_bytes = [12_u8; 32];
        let mut store = EncryptedSqliteStore::open(
            &path,
            StorageKey::from_slice(&key_bytes).unwrap(),
        )
        .unwrap();
        let authorization = PersistedAuthorization {
            authorization_state: AuthorizationState::Ready,
            self_user_id: Some(9_001),
            auth_context: None,
        };
        store
            .save_private_metadata(
                AUTHORIZATION_METADATA_KEY,
                &serde_json::to_vec(&authorization).unwrap(),
                1,
            )
            .unwrap();
        drop(store);

        let error =
            create_persistent_client(path.to_str().unwrap(), &key_bytes).unwrap_err();
        assert!(error
            .to_string()
            .contains("missing self-user identity or MTProto auth-key material"));
    }
}
