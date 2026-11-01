use crate::{
    dc::{DcDirectory, DcEndpoint, DcError},
    srp::{PasswordSrpParameters, PasswordSrpProof, SrpError},
    tl::{TlError, TlReader, TlWriter},
};
use serde::{Deserialize, Serialize};
use std::net::IpAddr;
use thiserror::Error;

pub const MTPROTO_LAYER: i32 = 227;
pub const INVOKE_WITH_LAYER_CONSTRUCTOR: u32 = 0xda9b_0d0d;
pub const INIT_CONNECTION_CONSTRUCTOR: u32 = 0xc1cd_5ea9;
pub const HELP_GET_CONFIG_CONSTRUCTOR: u32 = 0xc4f9_186b;
pub const AUTH_SEND_CODE_CONSTRUCTOR: u32 = 0xa677_244f;
pub const CODE_SETTINGS_CONSTRUCTOR: u32 = 0xad25_3d78;
pub const AUTH_SIGN_IN_CONSTRUCTOR: u32 = 0x8d52_a951;
pub const AUTH_SIGN_UP_CONSTRUCTOR: u32 = 0xaac7_b717;
pub const AUTH_AUTHORIZATION_CONSTRUCTOR: u32 = 0x2ea2_c0d4;
pub const AUTH_AUTHORIZATION_SIGN_UP_REQUIRED_CONSTRUCTOR: u32 = 0x4474_7e9a;
pub const USER_EMPTY_CONSTRUCTOR: u32 = 0xd3bc_4b7a;
pub const USER_CONSTRUCTOR: u32 = 0xb1b8_cc83;
pub const REQ_PQ_CONSTRUCTOR: u32 = 0x6046_9778;
pub const REQ_PQ_MULTI_CONSTRUCTOR: u32 = 0xbe7e_8ef1;
pub const MSGS_ACK_CONSTRUCTOR: u32 = 0x62d6_b459;
pub const PONG_CONSTRUCTOR: u32 = 0x3477_73c5;
pub const PING_CONSTRUCTOR: u32 = 0x7abe_77ec;
pub const PING_DELAY_DISCONNECT_CONSTRUCTOR: u32 = 0xf342_7b8c;
pub const RPC_RESULT_CONSTRUCTOR: u32 = 0xf35c_6d01;
pub const RPC_DROP_ANSWER_CONSTRUCTOR: u32 = 0x58e4_a740;
pub const RPC_ANSWER_UNKNOWN_CONSTRUCTOR: u32 = 0x5e2a_d36e;
pub const RPC_ANSWER_DROPPED_RUNNING_CONSTRUCTOR: u32 = 0xcd78_e586;
pub const RPC_ANSWER_DROPPED_CONSTRUCTOR: u32 = 0xa43a_d8b7;
pub const DESTROY_AUTH_KEY_CONSTRUCTOR: u32 = 0xd143_5160;
pub const DESTROY_AUTH_KEY_OK_CONSTRUCTOR: u32 = 0xf660_e1d4;
pub const DESTROY_AUTH_KEY_NONE_CONSTRUCTOR: u32 = 0x0a9f_2259;
pub const DESTROY_AUTH_KEY_FAIL_CONSTRUCTOR: u32 = 0xea10_9b13;
pub const DESTROY_SESSION_CONSTRUCTOR: u32 = 0xe751_2126;
pub const DESTROY_SESSION_OK_CONSTRUCTOR: u32 = 0xe220_45fc;
pub const DESTROY_SESSION_NONE_CONSTRUCTOR: u32 = 0x62d3_50c9;
pub const HTTP_WAIT_CONSTRUCTOR: u32 = 0x9299_359f;
pub const IP_PORT_CONSTRUCTOR: u32 = 0xd433_ad73;
pub const IP_PORT_SECRET_CONSTRUCTOR: u32 = 0x3798_2646;
pub const ACCESS_POINT_RULE_CONSTRUCTOR: u32 = 0x4679_b65f;
pub const HELP_CONFIG_SIMPLE_CONSTRUCTOR: u32 = 0x5a59_2a6c;
pub const MESSAGE_CONTAINER_CONSTRUCTOR: u32 = 0x73f1_f8dc;
pub const MAX_MESSAGE_CONTAINER_ENTRIES: usize = 1024;
pub const MSG_RESEND_REQ_CONSTRUCTOR: u32 = 0x7d86_1a08;
pub const MSGS_STATE_REQ_CONSTRUCTOR: u32 = 0xda69_fb52;
pub const MSGS_STATE_INFO_CONSTRUCTOR: u32 = 0x04de_b57d;
pub const MSGS_ALL_INFO_CONSTRUCTOR: u32 = 0x8cc0_d131;
pub const MSG_DETAILED_INFO_CONSTRUCTOR: u32 = 0x276d_3ec6;
pub const MSG_NEW_DETAILED_INFO_CONSTRUCTOR: u32 = 0x809d_b6df;
pub const BAD_MSG_NOTIFICATION_CONSTRUCTOR: u32 = 0xa7ef_f811;
pub const BAD_SERVER_SALT_CONSTRUCTOR: u32 = 0xedab_447b;
pub const FUTURE_SALT_CONSTRUCTOR: u32 = 0x0949_d9dc;
pub const FUTURE_SALTS_CONSTRUCTOR: u32 = 0xae50_0895;
pub const GET_FUTURE_SALTS_CONSTRUCTOR: u32 = 0xb921_bd04;
pub const CONFIG_CONSTRUCTOR: u32 = 0xcc1a_241e;
pub const DC_OPTION_CONSTRUCTOR: u32 = 0x18b7_a10d;
pub const AUTH_SENT_CODE_CONSTRUCTOR: u32 = 0x5e00_2502;
pub const AUTH_SENT_CODE_SUCCESS_CONSTRUCTOR: u32 = 0x2390_fe44;
pub const AUTH_SENT_CODE_PAYMENT_REQUIRED_CONSTRUCTOR: u32 = 0xf882_7ebf;
pub const RPC_ERROR_CONSTRUCTOR: u32 = 0x2144_ca19;
pub const ACCOUNT_GET_PASSWORD_CONSTRUCTOR: u32 = 0x548a_30f5;
pub const ACCOUNT_PASSWORD_CONSTRUCTOR: u32 = 0x957b_50fb;
pub const PASSWORD_KDF_ALGO_CONSTRUCTOR: u32 = 0x3a91_2d4a;
pub const AUTH_CHECK_PASSWORD_CONSTRUCTOR: u32 = 0xd18b_4d16;
pub const AUTH_REQUEST_PASSWORD_RECOVERY_CONSTRUCTOR: u32 = 0xd897_bc66;
pub const AUTH_PASSWORD_RECOVERY_CONSTRUCTOR: u32 = 0x1379_48a5;
pub const AUTH_CHECK_RECOVERY_PASSWORD_CONSTRUCTOR: u32 = 0x0d36_bf79;
pub const AUTH_RESEND_CODE_CONSTRUCTOR: u32 = 0xcae4_7523;
pub const AUTH_CANCEL_CODE_CONSTRUCTOR: u32 = 0x1f04_0578;
pub const ACCOUNT_SEND_VERIFY_EMAIL_CODE_CONSTRUCTOR: u32 = 0x98e0_37bb;
pub const ACCOUNT_VERIFY_EMAIL_CONSTRUCTOR: u32 = 0x032d_a4cf;
pub const EMAIL_VERIFY_PURPOSE_LOGIN_SETUP_CONSTRUCTOR: u32 = 0x4345_be73;
pub const EMAIL_VERIFICATION_CODE_CONSTRUCTOR: u32 = 0x922e_55a9;
pub const ACCOUNT_SENT_EMAIL_CODE_CONSTRUCTOR: u32 = 0x811f_854f;
pub const ACCOUNT_EMAIL_VERIFIED_CONSTRUCTOR: u32 = 0x2b96_cd1b;
pub const ACCOUNT_EMAIL_VERIFIED_LOGIN_CONSTRUCTOR: u32 = 0xe1bb_0d61;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InitConnection<'a> {
    pub api_id: i32,
    pub device_model: &'a str,
    pub system_version: &'a str,
    pub app_version: &'a str,
    pub system_lang_code: &'a str,
    pub lang_pack: &'a str,
    pub lang_code: &'a str,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfigDcDirectory {
    pub flags: i32,
    pub date: i32,
    pub expires: i32,
    pub directory: DcDirectory,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfigDcOptions {
    pub endpoints: Vec<DcEndpoint>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SentCode {
    pub phone_code_hash: String,
    pub delivery: SentCodeDelivery,
    pub next_type: Option<NextCodeType>,
    pub timeout_seconds: Option<i32>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", tag = "type")]
pub enum SentCodeResult {
    Code {
        code: SentCode,
    },
    Success {
        user_id: i64,
    },
    PaymentRequired {
        store_product: String,
        phone_code_hash: String,
        support_email_address: String,
        support_email_subject: String,
        premium_days: i32,
        currency: String,
        amount: i64,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", tag = "type")]
pub enum SentCodeDelivery {
    App { length: i32 },
    Sms { length: i32 },
    Call { length: i32 },
    FlashCall { pattern: String },
    MissedCall { prefix: String, length: i32 },
    Email { pattern: String, length: i32 },
    SetUpEmailRequired,
    FragmentSms { url: String, length: i32 },
    FirebaseSms { length: i32 },
    SmsWord { beginning: Option<String> },
    SmsPhrase { beginning: Option<String> },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum NextCodeType {
    Sms,
    Call,
    FlashCall,
    MissedCall,
    FragmentSms,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RpcErrorResponse {
    pub code: i32,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PasswordRecovery {
    pub email_pattern: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SentEmailCode {
    pub email_pattern: String,
    pub length: i32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", tag = "type")]
pub enum EmailVerifiedResult {
    LoginCode { email: String, code: SentCode },
    LoginSuccess { email: String, user_id: i64 },
    LoginPaymentRequired {
        email: String,
        store_product: String,
        phone_code_hash: String,
        support_email_address: String,
        support_email_subject: String,
        premium_days: i32,
        currency: String,
        amount: i64,
    },
    Verified { email: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AccountPasswordState {
    pub has_recovery: bool,
    pub hint: Option<String>,
    pub email_unconfirmed_pattern: Option<String>,
    pub srp: Option<PasswordSrpParameters>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MsgsStateInfo {
    pub request_message_id: i64,
    pub states: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MsgsAllInfo {
    pub message_ids: Vec<i64>,
    pub states: Vec<u8>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MsgDetailedInfo {
    pub message_id: i64,
    pub answer_message_id: i64,
    pub bytes: i32,
    pub status: i32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MsgNewDetailedInfo {
    pub answer_message_id: i64,
    pub bytes: i32,
    pub status: i32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BadMsgNotification {
    pub bad_message_id: i64,
    pub bad_message_sequence_number: i32,
    pub error_code: i32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BadServerSalt {
    pub bad_message_id: i64,
    pub bad_message_sequence_number: i32,
    pub error_code: i32,
    pub new_server_salt: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FutureSalt {
    pub valid_since: i32,
    pub valid_until: i32,
    pub salt: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FutureSalts {
    pub request_message_id: i64,
    pub now: i32,
    pub salts: Vec<FutureSalt>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Pong {
    pub message_id: i64,
    pub ping_id: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RpcResultEnvelope {
    pub request_message_id: i64,
    pub result_body: Vec<u8>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RpcDropAnswer {
    Unknown,
    DroppedRunning,
    Dropped {
        message_id: i64,
        sequence_number: i32,
        bytes: i32,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DestroyAuthKeyResult {
    Ok,
    None,
    Fail,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DestroySessionResult {
    Ok { session_id: i64 },
    None { session_id: i64 },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HttpWait {
    pub max_delay: i32,
    pub wait_after: i32,
    pub max_wait: i32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IpPort {
    Plain {
        ipv4: i32,
        port: i32,
    },
    Secret {
        ipv4: i32,
        port: i32,
        secret: Vec<u8>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AccessPointRule {
    pub phone_prefix_rules: String,
    pub dc_id: i32,
    pub ips: Vec<IpPort>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HelpConfigSimple {
    pub date: i32,
    pub expires: i32,
    pub rules: Vec<AccessPointRule>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MessageContainerEntry {
    pub message_id: i64,
    pub sequence_number: i32,
    pub body: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MessageContainer {
    pub entries: Vec<MessageContainerEntry>,
}

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum ApiRequestError {
    #[error("Telegram api_id must be positive")]
    InvalidApiId,
    #[error("Telegram API field {0} must not be empty")]
    EmptyField(&'static str),
    #[error("Telegram phone number must use international +number format")]
    InvalidPhoneNumber,
    #[error("Telegram phone code must contain only digits")]
    InvalidPhoneCode,
    #[error("TL encoding failed: {0}")]
    Tl(#[from] TlError),
    #[error("Telegram MTProto message-state payload is malformed")]
    InvalidMessageState,
    #[error("Telegram MTProto rpc_result body has invalid length {0}")]
    InvalidRpcResultBodyLength(usize),
    #[error("Telegram MTProto RpcDropAnswer has unexpected constructor {0:#010x}")]
    UnexpectedRpcDropAnswerConstructor(u32),
    #[error("Telegram MTProto DestroyAuthKeyRes has unexpected constructor {0:#010x}")]
    UnexpectedDestroyAuthKeyConstructor(u32),
    #[error("Telegram MTProto DestroySessionRes has unexpected constructor {0:#010x}")]
    UnexpectedDestroySessionConstructor(u32),
    #[error("Telegram MTProto IpPort has unexpected constructor {0:#010x}")]
    UnexpectedIpPortConstructor(u32),
    #[error("Telegram MTProto message container is malformed")]
    InvalidMessageContainer,
    #[error("Telegram MTProto future-salts payload is malformed")]
    InvalidFutureSalts,
    #[error("Telegram Config contains an invalid IP address: {0}")]
    InvalidIpAddress(String),
    #[error("Telegram Config contains an invalid port: {0}")]
    InvalidPort(i32),
    #[error("Telegram data center configuration failed: {0}")]
    Dc(#[from] DcError),
    #[error("Telegram API response contains {0} trailing bytes")]
    TrailingBytes(usize),
    #[error("Telegram password SRP failed: {0}")]
    Srp(#[from] SrpError),
}

pub fn build_help_get_config() -> Vec<u8> {
    let mut writer = TlWriter::new();
    writer.write_u32(HELP_GET_CONFIG_CONSTRUCTOR);
    writer.into_bytes()
}

pub fn build_init_connection_get_config(
    connection: &InitConnection<'_>,
) -> Result<Vec<u8>, ApiRequestError> {
    validate_connection(connection)?;
    let mut writer = TlWriter::new();
    writer.write_u32(INVOKE_WITH_LAYER_CONSTRUCTOR);
    writer.write_i32(MTPROTO_LAYER);
    writer.write_u32(INIT_CONNECTION_CONSTRUCTOR);
    writer.write_i32(0);
    writer.write_i32(connection.api_id);
    writer.write_string(connection.device_model)?;
    writer.write_string(connection.system_version)?;
    writer.write_string(connection.app_version)?;
    writer.write_string(connection.system_lang_code)?;
    writer.write_string(connection.lang_pack)?;
    writer.write_string(connection.lang_code)?;
    writer.write_u32(HELP_GET_CONFIG_CONSTRUCTOR);
    Ok(writer.into_bytes())
}

pub fn build_auth_send_code(
    phone_number: &str,
    api_id: i32,
    api_hash: &str,
) -> Result<Vec<u8>, ApiRequestError> {
    validate_credentials(api_id, api_hash)?;
    validate_phone_number(phone_number)?;
    let mut writer = TlWriter::new();
    writer.write_u32(AUTH_SEND_CODE_CONSTRUCTOR);
    writer.write_string(phone_number)?;
    writer.write_i32(api_id);
    writer.write_string(api_hash)?;
    writer.write_u32(CODE_SETTINGS_CONSTRUCTOR);
    writer.write_i32(0);
    Ok(writer.into_bytes())
}

pub fn build_auth_sign_in(
    phone_number: &str,
    phone_code_hash: &str,
    phone_code: &str,
) -> Result<Vec<u8>, ApiRequestError> {
    validate_phone_number(phone_number)?;
    require_nonempty(phone_code_hash, "phone_code_hash")?;
    if phone_code.is_empty() || !phone_code.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(ApiRequestError::InvalidPhoneCode);
    }
    let mut writer = TlWriter::new();
    writer.write_u32(AUTH_SIGN_IN_CONSTRUCTOR);
    writer.write_i32(1);
    writer.write_string(phone_number)?;
    writer.write_string(phone_code_hash)?;
    writer.write_string(phone_code)?;
    Ok(writer.into_bytes())
}

pub fn build_auth_sign_up(
    phone_number: &str,
    phone_code_hash: &str,
    first_name: &str,
    last_name: &str,
) -> Result<Vec<u8>, ApiRequestError> {
    validate_phone_number(phone_number)?;
    require_nonempty(phone_code_hash, "phone_code_hash")?;
    require_nonempty(first_name, "first_name")?;
    let mut writer = TlWriter::new();
    writer.write_u32(AUTH_SIGN_UP_CONSTRUCTOR);
    writer.write_i32(0);
    writer.write_string(phone_number)?;
    writer.write_string(phone_code_hash)?;
    writer.write_string(first_name.trim())?;
    writer.write_string(last_name.trim())?;
    Ok(writer.into_bytes())
}

pub fn build_auth_resend_code(
    phone_number: &str,
    phone_code_hash: &str,
    reason: Option<&str>,
) -> Result<Vec<u8>, ApiRequestError> {
    validate_phone_number(phone_number)?;
    require_nonempty(phone_code_hash, "phone_code_hash")?;
    let reason = reason.map(str::trim).filter(|value| !value.is_empty());
    let mut writer = TlWriter::new();
    writer.write_u32(AUTH_RESEND_CODE_CONSTRUCTOR);
    writer.write_i32(i32::from(reason.is_some()));
    writer.write_string(phone_number)?;
    writer.write_string(phone_code_hash)?;
    if let Some(reason) = reason {
        writer.write_string(reason)?;
    }
    Ok(writer.into_bytes())
}

pub fn build_auth_cancel_code(
    phone_number: &str,
    phone_code_hash: &str,
) -> Result<Vec<u8>, ApiRequestError> {
    validate_phone_number(phone_number)?;
    require_nonempty(phone_code_hash, "phone_code_hash")?;
    let mut writer = TlWriter::new();
    writer.write_u32(AUTH_CANCEL_CODE_CONSTRUCTOR);
    writer.write_string(phone_number)?;
    writer.write_string(phone_code_hash)?;
    Ok(writer.into_bytes())
}

pub fn build_auth_request_password_recovery() -> Vec<u8> {
    AUTH_REQUEST_PASSWORD_RECOVERY_CONSTRUCTOR.to_le_bytes().to_vec()
}

pub fn build_auth_check_recovery_password(code: &str) -> Result<Vec<u8>, ApiRequestError> {
    require_nonempty(code, "recovery_code")?;
    let mut writer = TlWriter::new();
    writer.write_u32(AUTH_CHECK_RECOVERY_PASSWORD_CONSTRUCTOR);
    writer.write_string(code.trim())?;
    Ok(writer.into_bytes())
}

pub fn build_account_send_verify_email_code(
    phone_number: &str,
    phone_code_hash: &str,
    email: &str,
) -> Result<Vec<u8>, ApiRequestError> {
    validate_phone_number(phone_number)?;
    require_nonempty(phone_code_hash, "phone_code_hash")?;
    require_nonempty(email, "email")?;
    let mut writer = TlWriter::new();
    writer.write_u32(ACCOUNT_SEND_VERIFY_EMAIL_CODE_CONSTRUCTOR);
    writer.write_u32(EMAIL_VERIFY_PURPOSE_LOGIN_SETUP_CONSTRUCTOR);
    writer.write_string(phone_number)?;
    writer.write_string(phone_code_hash)?;
    writer.write_string(email.trim())?;
    Ok(writer.into_bytes())
}

pub fn build_account_verify_email(
    phone_number: &str,
    phone_code_hash: &str,
    code: &str,
) -> Result<Vec<u8>, ApiRequestError> {
    validate_phone_number(phone_number)?;
    require_nonempty(phone_code_hash, "phone_code_hash")?;
    require_nonempty(code, "email_code")?;
    let mut writer = TlWriter::new();
    writer.write_u32(ACCOUNT_VERIFY_EMAIL_CONSTRUCTOR);
    writer.write_u32(EMAIL_VERIFY_PURPOSE_LOGIN_SETUP_CONSTRUCTOR);
    writer.write_string(phone_number)?;
    writer.write_string(phone_code_hash)?;
    writer.write_u32(EMAIL_VERIFICATION_CODE_CONSTRUCTOR);
    writer.write_string(code.trim())?;
    Ok(writer.into_bytes())
}

pub fn build_account_get_password() -> Vec<u8> {
    ACCOUNT_GET_PASSWORD_CONSTRUCTOR.to_le_bytes().to_vec()
}

pub fn build_auth_check_password(proof: &PasswordSrpProof) -> Result<Vec<u8>, ApiRequestError> {
    let encoded_proof = proof.encode_input_check_password()?;
    let mut output = Vec::with_capacity(4 + encoded_proof.len());
    output.extend_from_slice(&AUTH_CHECK_PASSWORD_CONSTRUCTOR.to_le_bytes());
    output.extend_from_slice(&encoded_proof);
    Ok(output)
}

pub fn build_req_pq(nonce: &[u8; 16]) -> Vec<u8> {
    let mut writer = TlWriter::new();
    writer.write_u32(REQ_PQ_CONSTRUCTOR);
    writer.write_i128_bytes(nonce);
    writer.into_bytes()
}

pub fn build_req_pq_multi(nonce: &[u8; 16]) -> Vec<u8> {
    let mut writer = TlWriter::with_capacity(20);
    writer.write_u32(REQ_PQ_MULTI_CONSTRUCTOR);
    writer.write_i128_bytes(nonce);
    writer.into_bytes()
}

pub fn build_rpc_drop_answer(req_msg_id: i64) -> Vec<u8> {
    let mut writer = TlWriter::new();
    writer.write_u32(RPC_DROP_ANSWER_CONSTRUCTOR);
    writer.write_i64(req_msg_id);
    writer.into_bytes()
}

pub fn build_ping(ping_id: i64) -> Vec<u8> {
    let mut writer = TlWriter::new();
    writer.write_u32(PING_CONSTRUCTOR);
    writer.write_i64(ping_id);
    writer.into_bytes()
}

pub fn build_ping_delay_disconnect(ping_id: i64, disconnect_delay: i32) -> Vec<u8> {
    let mut writer = TlWriter::new();
    writer.write_u32(PING_DELAY_DISCONNECT_CONSTRUCTOR);
    writer.write_i64(ping_id);
    writer.write_i32(disconnect_delay);
    writer.into_bytes()
}

pub fn build_destroy_auth_key() -> Vec<u8> {
    let mut writer = TlWriter::new();
    writer.write_u32(DESTROY_AUTH_KEY_CONSTRUCTOR);
    writer.into_bytes()
}

pub fn build_destroy_session(session_id: i64) -> Vec<u8> {
    let mut writer = TlWriter::new();
    writer.write_u32(DESTROY_SESSION_CONSTRUCTOR);
    writer.write_i64(session_id);
    writer.into_bytes()
}

pub fn build_get_future_salts(num: i32) -> Vec<u8> {
    let mut writer = TlWriter::new();
    writer.write_u32(GET_FUTURE_SALTS_CONSTRUCTOR);
    writer.write_i32(num);
    writer.into_bytes()
}

pub fn build_msgs_ack(message_ids: &[i64]) -> Result<Vec<u8>, ApiRequestError> {
    let mut writer = TlWriter::new();
    writer.write_u32(MSGS_ACK_CONSTRUCTOR);
    writer.write_vector_length(message_ids.len())?;
    for message_id in message_ids {
        writer.write_i64(*message_id);
    }
    Ok(writer.into_bytes())
}

pub fn parse_msgs_ack(input: &[u8]) -> Result<Vec<i64>, ApiRequestError> {
    let mut reader = TlReader::new(input);
    let constructor = reader.read_u32()?;
    if constructor != MSGS_ACK_CONSTRUCTOR {
        return Err(ApiRequestError::Tl(TlError::UnexpectedConstructor {
            expected: MSGS_ACK_CONSTRUCTOR,
            actual: constructor,
        }));
    }
    let count = reader.read_vector_length()?;
    let mut message_ids = Vec::with_capacity(count);
    for _ in 0..count {
        message_ids.push(reader.read_i64()?);
    }
    if !reader.is_finished() {
        return Err(ApiRequestError::TrailingBytes(reader.remaining()));
    }
    Ok(message_ids)
}

pub fn parse_pong(input: &[u8]) -> Result<Pong, ApiRequestError> {
    let mut reader = TlReader::new(input);
    let constructor = reader.read_u32()?;
    if constructor != PONG_CONSTRUCTOR {
        return Err(ApiRequestError::Tl(TlError::UnexpectedConstructor {
            expected: PONG_CONSTRUCTOR,
            actual: constructor,
        }));
    }
    let pong = Pong {
        message_id: reader.read_i64()?,
        ping_id: reader.read_i64()?,
    };
    if !reader.is_finished() {
        return Err(ApiRequestError::TrailingBytes(reader.remaining()));
    }
    Ok(pong)
}

pub fn parse_rpc_result(input: &[u8]) -> Result<RpcResultEnvelope, ApiRequestError> {
    let mut reader = TlReader::new(input);
    let constructor = reader.read_u32()?;
    if constructor != RPC_RESULT_CONSTRUCTOR {
        return Err(ApiRequestError::Tl(TlError::UnexpectedConstructor {
            expected: RPC_RESULT_CONSTRUCTOR,
            actual: constructor,
        }));
    }
    let request_message_id = reader.read_i64()?;
    let result_length = reader.remaining();
    if result_length < 4 || !result_length.is_multiple_of(4) {
        return Err(ApiRequestError::InvalidRpcResultBodyLength(result_length));
    }
    let result_body = input[reader.position()..].to_vec();
    Ok(RpcResultEnvelope {
        request_message_id,
        result_body,
    })
}

pub fn parse_rpc_drop_answer(input: &[u8]) -> Result<RpcDropAnswer, ApiRequestError> {
    let mut reader = TlReader::new(input);
    let constructor = reader.read_u32()?;
    let answer = match constructor {
        RPC_ANSWER_UNKNOWN_CONSTRUCTOR => RpcDropAnswer::Unknown,
        RPC_ANSWER_DROPPED_RUNNING_CONSTRUCTOR => RpcDropAnswer::DroppedRunning,
        RPC_ANSWER_DROPPED_CONSTRUCTOR => RpcDropAnswer::Dropped {
            message_id: reader.read_i64()?,
            sequence_number: reader.read_i32()?,
            bytes: reader.read_i32()?,
        },
        actual => return Err(ApiRequestError::UnexpectedRpcDropAnswerConstructor(actual)),
    };
    if !reader.is_finished() {
        return Err(ApiRequestError::TrailingBytes(reader.remaining()));
    }
    Ok(answer)
}

pub fn parse_destroy_auth_key_result(
    input: &[u8],
) -> Result<DestroyAuthKeyResult, ApiRequestError> {
    let mut reader = TlReader::new(input);
    let result = match reader.read_u32()? {
        DESTROY_AUTH_KEY_OK_CONSTRUCTOR => DestroyAuthKeyResult::Ok,
        DESTROY_AUTH_KEY_NONE_CONSTRUCTOR => DestroyAuthKeyResult::None,
        DESTROY_AUTH_KEY_FAIL_CONSTRUCTOR => DestroyAuthKeyResult::Fail,
        actual => return Err(ApiRequestError::UnexpectedDestroyAuthKeyConstructor(actual)),
    };
    if !reader.is_finished() {
        return Err(ApiRequestError::TrailingBytes(reader.remaining()));
    }
    Ok(result)
}

pub fn parse_destroy_session_result(
    input: &[u8],
) -> Result<DestroySessionResult, ApiRequestError> {
    let mut reader = TlReader::new(input);
    let constructor = reader.read_u32()?;
    let session_id = reader.read_i64()?;
    let result = match constructor {
        DESTROY_SESSION_OK_CONSTRUCTOR => DestroySessionResult::Ok { session_id },
        DESTROY_SESSION_NONE_CONSTRUCTOR => DestroySessionResult::None { session_id },
        actual => return Err(ApiRequestError::UnexpectedDestroySessionConstructor(actual)),
    };
    if !reader.is_finished() {
        return Err(ApiRequestError::TrailingBytes(reader.remaining()));
    }
    Ok(result)
}

pub fn parse_http_wait(input: &[u8]) -> Result<HttpWait, ApiRequestError> {
    let mut reader = TlReader::new(input);
    let constructor = reader.read_u32()?;
    if constructor != HTTP_WAIT_CONSTRUCTOR {
        return Err(ApiRequestError::Tl(TlError::UnexpectedConstructor {
            expected: HTTP_WAIT_CONSTRUCTOR,
            actual: constructor,
        }));
    }
    let wait = HttpWait {
        max_delay: reader.read_i32()?,
        wait_after: reader.read_i32()?,
        max_wait: reader.read_i32()?,
    };
    if !reader.is_finished() {
        return Err(ApiRequestError::TrailingBytes(reader.remaining()));
    }
    Ok(wait)
}

fn read_ip_port(reader: &mut TlReader<'_>) -> Result<IpPort, ApiRequestError> {
    match reader.read_u32()? {
        IP_PORT_CONSTRUCTOR => Ok(IpPort::Plain {
            ipv4: reader.read_i32()?,
            port: reader.read_i32()?,
        }),
        IP_PORT_SECRET_CONSTRUCTOR => Ok(IpPort::Secret {
            ipv4: reader.read_i32()?,
            port: reader.read_i32()?,
            secret: reader.read_bytes()?.to_vec(),
        }),
        actual => Err(ApiRequestError::UnexpectedIpPortConstructor(actual)),
    }
}

fn read_access_point_rule(reader: &mut TlReader<'_>) -> Result<AccessPointRule, ApiRequestError> {
    let constructor = reader.read_u32()?;
    if constructor != ACCESS_POINT_RULE_CONSTRUCTOR {
        return Err(ApiRequestError::Tl(TlError::UnexpectedConstructor {
            expected: ACCESS_POINT_RULE_CONSTRUCTOR,
            actual: constructor,
        }));
    }
    let phone_prefix_rules = reader.read_string()?.to_string();
    let dc_id = reader.read_i32()?;
    let count = reader.read_vector_length()?;
    let mut ips = Vec::with_capacity(count);
    for _ in 0..count {
        ips.push(read_ip_port(reader)?);
    }
    Ok(AccessPointRule {
        phone_prefix_rules,
        dc_id,
        ips,
    })
}

fn read_help_config_simple(reader: &mut TlReader<'_>) -> Result<HelpConfigSimple, ApiRequestError> {
    let constructor = reader.read_u32()?;
    if constructor != HELP_CONFIG_SIMPLE_CONSTRUCTOR {
        return Err(ApiRequestError::Tl(TlError::UnexpectedConstructor {
            expected: HELP_CONFIG_SIMPLE_CONSTRUCTOR,
            actual: constructor,
        }));
    }
    let date = reader.read_i32()?;
    let expires = reader.read_i32()?;
    let count = reader.read_vector_length()?;
    let mut rules = Vec::with_capacity(count);
    for _ in 0..count {
        rules.push(read_access_point_rule(reader)?);
    }
    Ok(HelpConfigSimple {
        date,
        expires,
        rules,
    })
}

pub fn parse_ip_port(input: &[u8]) -> Result<IpPort, ApiRequestError> {
    let mut reader = TlReader::new(input);
    let result = read_ip_port(&mut reader)?;
    if !reader.is_finished() {
        return Err(ApiRequestError::TrailingBytes(reader.remaining()));
    }
    Ok(result)
}

pub fn parse_access_point_rule(input: &[u8]) -> Result<AccessPointRule, ApiRequestError> {
    let mut reader = TlReader::new(input);
    let result = read_access_point_rule(&mut reader)?;
    if !reader.is_finished() {
        return Err(ApiRequestError::TrailingBytes(reader.remaining()));
    }
    Ok(result)
}

pub fn parse_help_config_simple(input: &[u8]) -> Result<HelpConfigSimple, ApiRequestError> {
    let mut reader = TlReader::new(input);
    let result = read_help_config_simple(&mut reader)?;
    if !reader.is_finished() {
        return Err(ApiRequestError::TrailingBytes(reader.remaining()));
    }
    Ok(result)
}

pub fn parse_message_container(input: &[u8]) -> Result<MessageContainer, ApiRequestError> {
    let mut reader = TlReader::new(input);
    let constructor = reader.read_u32()?;
    if constructor != MESSAGE_CONTAINER_CONSTRUCTOR {
        return Err(ApiRequestError::Tl(TlError::UnexpectedConstructor {
            expected: MESSAGE_CONTAINER_CONSTRUCTOR,
            actual: constructor,
        }));
    }
    let count = reader.read_i32()?;
    let count = usize::try_from(count).map_err(|_| ApiRequestError::InvalidMessageContainer)?;
    if count > MAX_MESSAGE_CONTAINER_ENTRIES {
        return Err(ApiRequestError::InvalidMessageContainer);
    }
    let mut entries = Vec::with_capacity(count);
    for _ in 0..count {
        let message_id = reader.read_i64()?;
        let sequence_number = reader.read_i32()?;
        let body_length = reader.read_i32()?;
        let body_length =
            usize::try_from(body_length).map_err(|_| ApiRequestError::InvalidMessageContainer)?;
        if body_length < 4 || !body_length.is_multiple_of(4) || body_length > reader.remaining() {
            return Err(ApiRequestError::InvalidMessageContainer);
        }
        let start = reader.position();
        let end = start
            .checked_add(body_length)
            .ok_or(ApiRequestError::InvalidMessageContainer)?;
        let body = input
            .get(start..end)
            .ok_or(ApiRequestError::InvalidMessageContainer)?
            .to_vec();
        for _ in 0..(body_length / 4) {
            reader.read_u32()?;
        }
        entries.push(MessageContainerEntry {
            message_id,
            sequence_number,
            body,
        });
    }
    if !reader.is_finished() {
        return Err(ApiRequestError::InvalidMessageContainer);
    }
    Ok(MessageContainer { entries })
}

pub fn build_msg_resend_req(message_ids: &[i64]) -> Result<Vec<u8>, ApiRequestError> {
    let mut writer = TlWriter::new();
    writer.write_u32(MSG_RESEND_REQ_CONSTRUCTOR);
    writer.write_vector_length(message_ids.len())?;
    for message_id in message_ids {
        writer.write_i64(*message_id);
    }
    Ok(writer.into_bytes())
}

pub fn build_msgs_state_req(message_ids: &[i64]) -> Result<Vec<u8>, ApiRequestError> {
    let mut writer = TlWriter::new();
    writer.write_u32(MSGS_STATE_REQ_CONSTRUCTOR);
    writer.write_vector_length(message_ids.len())?;
    for message_id in message_ids {
        writer.write_i64(*message_id);
    }
    Ok(writer.into_bytes())
}

pub fn parse_msgs_state_info(input: &[u8]) -> Result<MsgsStateInfo, ApiRequestError> {
    let mut reader = TlReader::new(input);
    let constructor = reader.read_u32()?;
    if constructor != MSGS_STATE_INFO_CONSTRUCTOR {
        return Err(ApiRequestError::Tl(TlError::UnexpectedConstructor {
            expected: MSGS_STATE_INFO_CONSTRUCTOR,
            actual: constructor,
        }));
    }
    let request_message_id = reader.read_i64()?;
    let states = reader.read_bytes()?.to_vec();
    if !reader.is_finished() {
        return Err(ApiRequestError::InvalidMessageState);
    }
    Ok(MsgsStateInfo {
        request_message_id,
        states,
    })
}

pub fn parse_msgs_all_info(input: &[u8]) -> Result<MsgsAllInfo, ApiRequestError> {
    let mut reader = TlReader::new(input);
    let constructor = reader.read_u32()?;
    if constructor != MSGS_ALL_INFO_CONSTRUCTOR {
        return Err(ApiRequestError::Tl(TlError::UnexpectedConstructor {
            expected: MSGS_ALL_INFO_CONSTRUCTOR,
            actual: constructor,
        }));
    }
    let count = reader.read_vector_length()?;
    let mut message_ids = Vec::with_capacity(count);
    for _ in 0..count {
        message_ids.push(reader.read_i64()?);
    }
    let states = reader.read_bytes()?.to_vec();
    if !reader.is_finished() || states.len() != message_ids.len() {
        return Err(ApiRequestError::InvalidMessageState);
    }
    Ok(MsgsAllInfo {
        message_ids,
        states,
    })
}

pub fn parse_msg_detailed_info(input: &[u8]) -> Result<MsgDetailedInfo, ApiRequestError> {
    let mut reader = TlReader::new(input);
    let constructor = reader.read_u32()?;
    if constructor != MSG_DETAILED_INFO_CONSTRUCTOR {
        return Err(ApiRequestError::Tl(TlError::UnexpectedConstructor {
            expected: MSG_DETAILED_INFO_CONSTRUCTOR,
            actual: constructor,
        }));
    }
    let info = MsgDetailedInfo {
        message_id: reader.read_i64()?,
        answer_message_id: reader.read_i64()?,
        bytes: reader.read_i32()?,
        status: reader.read_i32()?,
    };
    if !reader.is_finished() {
        return Err(ApiRequestError::TrailingBytes(reader.remaining()));
    }
    Ok(info)
}

pub fn parse_msg_new_detailed_info(
    input: &[u8],
) -> Result<MsgNewDetailedInfo, ApiRequestError> {
    let mut reader = TlReader::new(input);
    let constructor = reader.read_u32()?;
    if constructor != MSG_NEW_DETAILED_INFO_CONSTRUCTOR {
        return Err(ApiRequestError::Tl(TlError::UnexpectedConstructor {
            expected: MSG_NEW_DETAILED_INFO_CONSTRUCTOR,
            actual: constructor,
        }));
    }
    let info = MsgNewDetailedInfo {
        answer_message_id: reader.read_i64()?,
        bytes: reader.read_i32()?,
        status: reader.read_i32()?,
    };
    if !reader.is_finished() {
        return Err(ApiRequestError::TrailingBytes(reader.remaining()));
    }
    Ok(info)
}

pub fn parse_bad_msg_notification(input: &[u8]) -> Result<BadMsgNotification, ApiRequestError> {
    let mut reader = TlReader::new(input);
    let constructor = reader.read_u32()?;
    if constructor != BAD_MSG_NOTIFICATION_CONSTRUCTOR {
        return Err(ApiRequestError::Tl(TlError::UnexpectedConstructor {
            expected: BAD_MSG_NOTIFICATION_CONSTRUCTOR,
            actual: constructor,
        }));
    }
    let notification = BadMsgNotification {
        bad_message_id: reader.read_i64()?,
        bad_message_sequence_number: reader.read_i32()?,
        error_code: reader.read_i32()?,
    };
    if !reader.is_finished() {
        return Err(ApiRequestError::TrailingBytes(reader.remaining()));
    }
    Ok(notification)
}

pub fn parse_bad_server_salt(input: &[u8]) -> Result<BadServerSalt, ApiRequestError> {
    let mut reader = TlReader::new(input);
    let constructor = reader.read_u32()?;
    if constructor != BAD_SERVER_SALT_CONSTRUCTOR {
        return Err(ApiRequestError::Tl(TlError::UnexpectedConstructor {
            expected: BAD_SERVER_SALT_CONSTRUCTOR,
            actual: constructor,
        }));
    }
    let notification = BadServerSalt {
        bad_message_id: reader.read_i64()?,
        bad_message_sequence_number: reader.read_i32()?,
        error_code: reader.read_i32()?,
        new_server_salt: reader.read_i64()?,
    };
    if !reader.is_finished() {
        return Err(ApiRequestError::TrailingBytes(reader.remaining()));
    }
    Ok(notification)
}

pub fn parse_future_salts(input: &[u8]) -> Result<FutureSalts, ApiRequestError> {
    const FUTURE_SALT_WIRE_BYTES: usize = 20;

    let mut reader = TlReader::new(input);
    let constructor = reader.read_u32()?;
    if constructor != FUTURE_SALTS_CONSTRUCTOR {
        return Err(ApiRequestError::Tl(TlError::UnexpectedConstructor {
            expected: FUTURE_SALTS_CONSTRUCTOR,
            actual: constructor,
        }));
    }
    let request_message_id = reader.read_i64()?;
    let now = reader.read_i32()?;
    let count = reader.read_vector_length()?;
    let required_bytes = count
        .checked_mul(FUTURE_SALT_WIRE_BYTES)
        .ok_or(ApiRequestError::InvalidFutureSalts)?;
    if required_bytes > reader.remaining() {
        return Err(ApiRequestError::InvalidFutureSalts);
    }

    let mut salts = Vec::with_capacity(count);
    for _ in 0..count {
        let constructor = reader.read_u32()?;
        if constructor != FUTURE_SALT_CONSTRUCTOR {
            return Err(ApiRequestError::Tl(TlError::UnexpectedConstructor {
                expected: FUTURE_SALT_CONSTRUCTOR,
                actual: constructor,
            }));
        }
        salts.push(FutureSalt {
            valid_since: reader.read_i32()?,
            valid_until: reader.read_i32()?,
            salt: reader.read_i64()?,
        });
    }
    if !reader.is_finished() {
        return Err(ApiRequestError::TrailingBytes(reader.remaining()));
    }
    Ok(FutureSalts {
        request_message_id,
        now,
        salts,
    })
}

pub fn parse_config_dc_options_prefix(
    input: &[u8],
) -> Result<ConfigDcOptions, ApiRequestError> {
    let mut reader = TlReader::new(input);
    let constructor = reader.read_u32()?;
    if constructor != CONFIG_CONSTRUCTOR {
        return Err(ApiRequestError::Tl(TlError::UnexpectedConstructor {
            expected: CONFIG_CONSTRUCTOR,
            actual: constructor,
        }));
    }
    let _flags = reader.read_i32()?;
    let _date = reader.read_i32()?;
    let _expires = reader.read_i32()?;
    let _test_mode = reader.read_bool()?;
    let _this_dc = reader.read_i32()?;
    let count = reader.read_vector_length()?;
    let mut endpoints = Vec::with_capacity(count);
    for _ in 0..count {
        let constructor = reader.read_u32()?;
        if constructor != DC_OPTION_CONSTRUCTOR {
            return Err(ApiRequestError::Tl(TlError::UnexpectedConstructor {
                expected: DC_OPTION_CONSTRUCTOR,
                actual: constructor,
            }));
        }
        let option_flags = reader.read_i32()?;
        let dc_id = reader.read_i32()?;
        let address = reader.read_string()?.to_string();
        let ip_address: IpAddr = address
            .parse()
            .map_err(|_| ApiRequestError::InvalidIpAddress(address))?;
        let raw_port = reader.read_i32()?;
        let port = u16::try_from(raw_port)
            .ok()
            .filter(|value| *value != 0)
            .ok_or(ApiRequestError::InvalidPort(raw_port))?;
        let secret = if option_flags & (1 << 10) != 0 {
            Some(reader.read_bytes()?.to_vec())
        } else {
            None
        };
        endpoints.push(DcEndpoint::new(
            dc_id,
            ip_address,
            port,
            option_flags & 2 != 0,
            option_flags & 4 != 0,
            option_flags & 8 != 0,
            option_flags & 16 != 0,
            option_flags & 32 != 0,
            secret,
        )?);
    }
    if endpoints.is_empty() {
        return Err(ApiRequestError::Dc(DcError::EmptyDirectory));
    }
    Ok(ConfigDcOptions { endpoints })
}

/// Parses the stable prefix of Telegram `Config` through `dc_options`.
/// Remaining layer-specific Config fields are intentionally left unread.
pub fn parse_config_dc_directory_prefix(
    input: &[u8],
) -> Result<ConfigDcDirectory, ApiRequestError> {
    let mut reader = TlReader::new(input);
    let constructor = reader.read_u32()?;
    if constructor != CONFIG_CONSTRUCTOR {
        return Err(ApiRequestError::Tl(TlError::UnexpectedConstructor {
            expected: CONFIG_CONSTRUCTOR,
            actual: constructor,
        }));
    }
    let flags = reader.read_i32()?;
    let date = reader.read_i32()?;
    let expires = reader.read_i32()?;
    let test_mode = reader.read_bool()?;
    let this_dc = reader.read_i32()?;
    let count = reader.read_vector_length()?;
    let mut endpoints = Vec::with_capacity(count);
    for _ in 0..count {
        let constructor = reader.read_u32()?;
        if constructor != DC_OPTION_CONSTRUCTOR {
            return Err(ApiRequestError::Tl(TlError::UnexpectedConstructor {
                expected: DC_OPTION_CONSTRUCTOR,
                actual: constructor,
            }));
        }
        let option_flags = reader.read_i32()?;
        let dc_id = reader.read_i32()?;
        let address = reader.read_string()?.to_string();
        let ip_address: IpAddr = address
            .parse()
            .map_err(|_| ApiRequestError::InvalidIpAddress(address))?;
        let raw_port = reader.read_i32()?;
        let port = u16::try_from(raw_port)
            .ok()
            .filter(|value| *value != 0)
            .ok_or(ApiRequestError::InvalidPort(raw_port))?;
        let secret = if option_flags & (1 << 10) != 0 {
            Some(reader.read_bytes()?.to_vec())
        } else {
            None
        };
        endpoints.push(DcEndpoint::new(
            dc_id,
            ip_address,
            port,
            option_flags & 2 != 0,
            option_flags & 4 != 0,
            option_flags & 8 != 0,
            option_flags & 16 != 0,
            option_flags & 32 != 0,
            secret,
        )?);
    }
    Ok(ConfigDcDirectory {
        flags,
        date,
        expires,
        directory: DcDirectory::new(this_dc, test_mode, endpoints)?,
    })
}

pub fn parse_auth_sent_code(input: &[u8]) -> Result<SentCodeResult, ApiRequestError> {
    let mut reader = TlReader::new(input);
    let constructor = reader.read_u32()?;
    match constructor {
        AUTH_SENT_CODE_CONSTRUCTOR => {
            let flags = reader.read_i32()?;
            let delivery = parse_sent_code_delivery(&mut reader)?;
            let phone_code_hash = reader.read_string()?.to_string();
            let next_type = if flags & 2 != 0 {
                Some(parse_next_code_type(&mut reader)?)
            } else {
                None
            };
            let timeout_seconds = if flags & 4 != 0 {
                Some(reader.read_i32()?)
            } else {
                None
            };
            if !reader.is_finished() {
                return Err(ApiRequestError::TrailingBytes(reader.remaining()));
            }
            Ok(SentCodeResult::Code {
                code: SentCode {
                    phone_code_hash,
                    delivery,
                    next_type,
                    timeout_seconds,
                },
            })
        }
        AUTH_SENT_CODE_SUCCESS_CONSTRUCTOR => Ok(SentCodeResult::Success {
            user_id: parse_auth_authorization_user_id_from_reader(&mut reader)?,
        }),
        AUTH_SENT_CODE_PAYMENT_REQUIRED_CONSTRUCTOR => {
            let result = SentCodeResult::PaymentRequired {
                store_product: reader.read_string()?.to_string(),
                phone_code_hash: reader.read_string()?.to_string(),
                support_email_address: reader.read_string()?.to_string(),
                support_email_subject: reader.read_string()?.to_string(),
                premium_days: reader.read_i32()?,
                currency: reader.read_string()?.to_string(),
                amount: reader.read_i64()?,
            };
            if !reader.is_finished() {
                return Err(ApiRequestError::TrailingBytes(reader.remaining()));
            }
            Ok(result)
        }
        actual => Err(ApiRequestError::Tl(TlError::UnexpectedConstructor {
            expected: AUTH_SENT_CODE_CONSTRUCTOR,
            actual,
        })),
    }
}

pub fn parse_bool(input: &[u8]) -> Result<bool, ApiRequestError> {
    let mut reader = TlReader::new(input);
    let value = reader.read_bool()?;
    if !reader.is_finished() {
        return Err(ApiRequestError::TrailingBytes(reader.remaining()));
    }
    Ok(value)
}

pub fn parse_auth_password_recovery(input: &[u8]) -> Result<PasswordRecovery, ApiRequestError> {
    let mut reader = TlReader::new(input);
    let constructor = reader.read_u32()?;
    if constructor != AUTH_PASSWORD_RECOVERY_CONSTRUCTOR {
        return Err(ApiRequestError::Tl(TlError::UnexpectedConstructor {
            expected: AUTH_PASSWORD_RECOVERY_CONSTRUCTOR,
            actual: constructor,
        }));
    }
    let recovery = PasswordRecovery {
        email_pattern: reader.read_string()?.to_string(),
    };
    if !reader.is_finished() {
        return Err(ApiRequestError::TrailingBytes(reader.remaining()));
    }
    Ok(recovery)
}

pub fn parse_account_sent_email_code(input: &[u8]) -> Result<SentEmailCode, ApiRequestError> {
    let mut reader = TlReader::new(input);
    let constructor = reader.read_u32()?;
    if constructor != ACCOUNT_SENT_EMAIL_CODE_CONSTRUCTOR {
        return Err(ApiRequestError::Tl(TlError::UnexpectedConstructor {
            expected: ACCOUNT_SENT_EMAIL_CODE_CONSTRUCTOR,
            actual: constructor,
        }));
    }
    let sent = SentEmailCode {
        email_pattern: reader.read_string()?.to_string(),
        length: reader.read_i32()?,
    };
    if !reader.is_finished() {
        return Err(ApiRequestError::TrailingBytes(reader.remaining()));
    }
    Ok(sent)
}

pub fn parse_account_email_verified(input: &[u8]) -> Result<EmailVerifiedResult, ApiRequestError> {
    let mut reader = TlReader::new(input);
    let constructor = reader.read_u32()?;
    match constructor {
        ACCOUNT_EMAIL_VERIFIED_CONSTRUCTOR => {
            let email = reader.read_string()?.to_string();
            if !reader.is_finished() {
                return Err(ApiRequestError::TrailingBytes(reader.remaining()));
            }
            Ok(EmailVerifiedResult::Verified { email })
        }
        ACCOUNT_EMAIL_VERIFIED_LOGIN_CONSTRUCTOR => {
            let email = reader.read_string()?.to_string();
            let sent_constructor = reader.read_u32()?;
            match sent_constructor {
                AUTH_SENT_CODE_CONSTRUCTOR => {
                    let flags = reader.read_i32()?;
                    let delivery = parse_sent_code_delivery(&mut reader)?;
                    let phone_code_hash = reader.read_string()?.to_string();
                    let next_type = if flags & 2 != 0 {
                        Some(parse_next_code_type(&mut reader)?)
                    } else {
                        None
                    };
                    let timeout_seconds = if flags & 4 != 0 {
                        Some(reader.read_i32()?)
                    } else {
                        None
                    };
                    if !reader.is_finished() {
                        return Err(ApiRequestError::TrailingBytes(reader.remaining()));
                    }
                    Ok(EmailVerifiedResult::LoginCode {
                        email,
                        code: SentCode {
                            phone_code_hash,
                            delivery,
                            next_type,
                            timeout_seconds,
                        },
                    })
                }
                AUTH_SENT_CODE_SUCCESS_CONSTRUCTOR => Ok(EmailVerifiedResult::LoginSuccess {
                    email,
                    user_id: parse_auth_authorization_user_id_from_reader(&mut reader)?,
                }),
                AUTH_SENT_CODE_PAYMENT_REQUIRED_CONSTRUCTOR => {
                    let result = EmailVerifiedResult::LoginPaymentRequired {
                        email,
                        store_product: reader.read_string()?.to_string(),
                        phone_code_hash: reader.read_string()?.to_string(),
                        support_email_address: reader.read_string()?.to_string(),
                        support_email_subject: reader.read_string()?.to_string(),
                        premium_days: reader.read_i32()?,
                        currency: reader.read_string()?.to_string(),
                        amount: reader.read_i64()?,
                    };
                    if !reader.is_finished() {
                        return Err(ApiRequestError::TrailingBytes(reader.remaining()));
                    }
                    Ok(result)
                }
                actual => Err(ApiRequestError::Tl(TlError::UnexpectedConstructor {
                    expected: AUTH_SENT_CODE_CONSTRUCTOR,
                    actual,
                })),
            }
        }
        actual => Err(ApiRequestError::Tl(TlError::UnexpectedConstructor {
            expected: ACCOUNT_EMAIL_VERIFIED_LOGIN_CONSTRUCTOR,
            actual,
        })),
    }
}

pub fn parse_auth_authorization_user_id(input: &[u8]) -> Result<i64, ApiRequestError> {
    let mut reader = TlReader::new(input);
    let user_id = parse_auth_authorization_user_id_from_reader(&mut reader)?;
    if !reader.is_finished() {
        return Err(ApiRequestError::TrailingBytes(reader.remaining()));
    }
    Ok(user_id)
}

fn parse_auth_authorization_user_id_from_reader(
    reader: &mut TlReader<'_>,
) -> Result<i64, ApiRequestError> {
    let constructor = reader.read_u32()?;
    if constructor != AUTH_AUTHORIZATION_CONSTRUCTOR {
        return Err(ApiRequestError::Tl(TlError::UnexpectedConstructor {
            expected: AUTH_AUTHORIZATION_CONSTRUCTOR,
            actual: constructor,
        }));
    }
    let flags = reader.read_i32()?;
    if flags & 1 != 0 {
        reader.read_i32()?;
    }
    if flags & 2 != 0 {
        reader.read_i32()?;
    }
    if flags & 4 != 0 {
        reader.read_bytes()?;
    }
    match reader.read_u32()? {
        USER_EMPTY_CONSTRUCTOR => Ok(reader.read_i64()?),
        USER_CONSTRUCTOR => {
            reader.read_i32()?;
            reader.read_i32()?;
            Ok(reader.read_i64()?)
        }
        actual => Err(ApiRequestError::Tl(TlError::UnexpectedConstructor {
            expected: USER_CONSTRUCTOR,
            actual,
        })),
    }
}

pub fn try_parse_rpc_error(input: &[u8]) -> Result<Option<RpcErrorResponse>, ApiRequestError> {
    if input.len() < 4 {
        return Err(ApiRequestError::Tl(TlError::UnexpectedEof {
            offset: input.len(),
            needed: 4 - input.len(),
        }));
    }
    if u32::from_le_bytes(input[..4].try_into().expect("four-byte prefix")) != RPC_ERROR_CONSTRUCTOR
    {
        return Ok(None);
    }
    let mut reader = TlReader::new(input);
    reader.read_u32()?;
    let error = RpcErrorResponse {
        code: reader.read_i32()?,
        message: reader.read_string()?.to_string(),
    };
    if !reader.is_finished() {
        return Err(ApiRequestError::TrailingBytes(reader.remaining()));
    }
    Ok(Some(error))
}

/// Parses the current-password prefix of `account.Password`. New-password and
/// secure-secret algorithms follow this prefix and belong to settings flows.
pub fn parse_account_password_prefix(
    input: &[u8],
) -> Result<AccountPasswordState, ApiRequestError> {
    let mut reader = TlReader::new(input);
    let constructor = reader.read_u32()?;
    if constructor != ACCOUNT_PASSWORD_CONSTRUCTOR {
        return Err(ApiRequestError::Tl(TlError::UnexpectedConstructor {
            expected: ACCOUNT_PASSWORD_CONSTRUCTOR,
            actual: constructor,
        }));
    }
    let flags = reader.read_i32()?;
    let srp = if flags & 4 != 0 {
        let algorithm = reader.read_u32()?;
        if algorithm != PASSWORD_KDF_ALGO_CONSTRUCTOR {
            return Err(ApiRequestError::Tl(TlError::UnexpectedConstructor {
                expected: PASSWORD_KDF_ALGO_CONSTRUCTOR,
                actual: algorithm,
            }));
        }
        let salt1 = reader.read_bytes()?.to_vec();
        let salt2 = reader.read_bytes()?.to_vec();
        let generator = reader.read_i32()?;
        let prime = reader.read_bytes()?.to_vec();
        let server_b = reader.read_bytes()?.to_vec();
        let srp_id = reader.read_i64()?;
        Some(PasswordSrpParameters {
            srp_id,
            salt1,
            salt2,
            generator,
            prime,
            server_b,
        })
    } else {
        None
    };
    let hint = if flags & 8 != 0 {
        Some(reader.read_string()?.to_string())
    } else {
        None
    };
    let email_unconfirmed_pattern = if flags & 16 != 0 {
        Some(reader.read_string()?.to_string())
    } else {
        None
    };
    Ok(AccountPasswordState {
        has_recovery: flags & 1 != 0,
        hint,
        email_unconfirmed_pattern,
        srp,
    })
}

fn parse_sent_code_delivery(
    reader: &mut TlReader<'_>,
) -> Result<SentCodeDelivery, ApiRequestError> {
    Ok(match reader.read_u32()? {
        0x3dbb_5986 => SentCodeDelivery::App {
            length: reader.read_i32()?,
        },
        0xc000_bba2 => SentCodeDelivery::Sms {
            length: reader.read_i32()?,
        },
        0x5353_e5a7 => SentCodeDelivery::Call {
            length: reader.read_i32()?,
        },
        0xab03_c6d9 => SentCodeDelivery::FlashCall {
            pattern: reader.read_string()?.to_string(),
        },
        0x8200_6484 => SentCodeDelivery::MissedCall {
            prefix: reader.read_string()?.to_string(),
            length: reader.read_i32()?,
        },
        0xf450_f59b => {
            let flags = reader.read_i32()?;
            let pattern = reader.read_string()?.to_string();
            let length = reader.read_i32()?;
            if flags & 8 != 0 {
                reader.read_i32()?;
            }
            if flags & 16 != 0 {
                reader.read_i32()?;
            }
            SentCodeDelivery::Email { pattern, length }
        }
        0xa549_1dea => {
            reader.read_i32()?;
            SentCodeDelivery::SetUpEmailRequired
        }
        0xd956_5c39 => SentCodeDelivery::FragmentSms {
            url: reader.read_string()?.to_string(),
            length: reader.read_i32()?,
        },
        0x009f_d736 => {
            let flags = reader.read_i32()?;
            if flags & 1 != 0 {
                reader.read_bytes()?;
            }
            if flags & 4 != 0 {
                reader.read_i64()?;
                reader.read_bytes()?;
            }
            if flags & 2 != 0 {
                reader.read_string()?;
                reader.read_i32()?;
            }
            SentCodeDelivery::FirebaseSms {
                length: reader.read_i32()?,
            }
        }
        0xa416_ac81 => SentCodeDelivery::SmsWord {
            beginning: read_optional_beginning(reader)?,
        },
        0xb377_94af => SentCodeDelivery::SmsPhrase {
            beginning: read_optional_beginning(reader)?,
        },
        actual => {
            return Err(ApiRequestError::Tl(TlError::UnexpectedConstructor {
                expected: 0x3dbb_5986,
                actual,
            }))
        }
    })
}

fn read_optional_beginning(reader: &mut TlReader<'_>) -> Result<Option<String>, ApiRequestError> {
    let flags = reader.read_i32()?;
    Ok(if flags & 1 != 0 {
        Some(reader.read_string()?.to_string())
    } else {
        None
    })
}

fn parse_next_code_type(reader: &mut TlReader<'_>) -> Result<NextCodeType, ApiRequestError> {
    match reader.read_u32()? {
        0x72a3_158c => Ok(NextCodeType::Sms),
        0x741c_d3e3 => Ok(NextCodeType::Call),
        0x226c_cefb => Ok(NextCodeType::FlashCall),
        0xd61a_d6ee => Ok(NextCodeType::MissedCall),
        0x06ed_998c => Ok(NextCodeType::FragmentSms),
        actual => Err(ApiRequestError::Tl(TlError::UnexpectedConstructor {
            expected: 0x72a3_158c,
            actual,
        })),
    }
}

fn validate_connection(connection: &InitConnection<'_>) -> Result<(), ApiRequestError> {
    if connection.api_id <= 0 {
        return Err(ApiRequestError::InvalidApiId);
    }
    require_nonempty(connection.device_model, "device_model")?;
    require_nonempty(connection.system_version, "system_version")?;
    require_nonempty(connection.app_version, "app_version")?;
    require_nonempty(connection.system_lang_code, "system_lang_code")?;
    require_nonempty(connection.lang_code, "lang_code")?;
    Ok(())
}

fn validate_credentials(api_id: i32, api_hash: &str) -> Result<(), ApiRequestError> {
    if api_id <= 0 {
        return Err(ApiRequestError::InvalidApiId);
    }
    require_nonempty(api_hash, "api_hash")
}

fn validate_phone_number(phone_number: &str) -> Result<(), ApiRequestError> {
    if phone_number.len() < 8
        || !phone_number.starts_with('+')
        || !phone_number[1..].bytes().all(|byte| byte.is_ascii_digit())
    {
        return Err(ApiRequestError::InvalidPhoneNumber);
    }
    Ok(())
}

fn require_nonempty(value: &str, field: &'static str) -> Result<(), ApiRequestError> {
    if value.trim().is_empty() {
        Err(ApiRequestError::EmptyField(field))
    } else {
        Ok(())
    }
}


#[cfg(test)]
mod authorization_identity_contract_tests {
    use super::*;

    fn authorization_with_user(user_id: i64) -> Vec<u8> {
        let mut writer = TlWriter::new();
        writer.write_u32(AUTH_AUTHORIZATION_CONSTRUCTOR);
        writer.write_i32(0);
        writer.write_u32(USER_CONSTRUCTOR);
        writer.write_i32(0);
        writer.write_i32(0);
        writer.write_i64(user_id);
        writer.into_bytes()
    }

    #[test]
    fn auth_authorization_exposes_canonical_self_user_id() {
        assert_eq!(
            parse_auth_authorization_user_id(&authorization_with_user(4_242)).unwrap(),
            4_242
        );
    }

    #[test]
    fn sent_code_success_carries_authorized_self_user_id() {
        let mut payload = AUTH_SENT_CODE_SUCCESS_CONSTRUCTOR.to_le_bytes().to_vec();
        payload.extend_from_slice(&authorization_with_user(9_001));
        assert_eq!(
            parse_auth_sent_code(&payload).unwrap(),
            SentCodeResult::Success { user_id: 9_001 }
        );
    }
}

#[cfg(test)]
mod mtproto_service_builder_contract_tests {
    use super::{
        build_destroy_auth_key, build_destroy_session, build_get_future_salts, build_msg_resend_req, build_msgs_ack,
        build_msgs_state_req, build_ping, build_ping_delay_disconnect, build_rpc_drop_answer,
        parse_bad_msg_notification,
        parse_bad_server_salt,
        parse_future_salts, parse_msg_detailed_info, parse_msg_new_detailed_info, parse_msgs_ack,
        parse_access_point_rule, parse_destroy_session_result, parse_help_config_simple,
        parse_http_wait, parse_ip_port, parse_message_container, parse_msgs_all_info,
        parse_msgs_state_info, parse_pong, parse_rpc_drop_answer, parse_rpc_result,
        AccessPointRule, IpPort, BAD_MSG_NOTIFICATION_CONSTRUCTOR, BAD_SERVER_SALT_CONSTRUCTOR,
        DESTROY_AUTH_KEY_CONSTRUCTOR, DESTROY_SESSION_CONSTRUCTOR, DESTROY_SESSION_NONE_CONSTRUCTOR,
        DESTROY_SESSION_OK_CONSTRUCTOR, DestroySessionResult,
        ACCESS_POINT_RULE_CONSTRUCTOR, FUTURE_SALTS_CONSTRUCTOR, GET_FUTURE_SALTS_CONSTRUCTOR,
        HELP_CONFIG_SIMPLE_CONSTRUCTOR,
        HTTP_WAIT_CONSTRUCTOR, IP_PORT_CONSTRUCTOR, IP_PORT_SECRET_CONSTRUCTOR, RpcDropAnswer,
        RPC_ANSWER_DROPPED_CONSTRUCTOR,
        RPC_ANSWER_DROPPED_RUNNING_CONSTRUCTOR, RPC_ANSWER_UNKNOWN_CONSTRUCTOR,
        RPC_DROP_ANSWER_CONSTRUCTOR, FUTURE_SALT_CONSTRUCTOR, MESSAGE_CONTAINER_CONSTRUCTOR, MSG_DETAILED_INFO_CONSTRUCTOR,
        MSG_NEW_DETAILED_INFO_CONSTRUCTOR, MSGS_ACK_CONSTRUCTOR,
        MSGS_ALL_INFO_CONSTRUCTOR,
        MSGS_STATE_INFO_CONSTRUCTOR, MSGS_STATE_REQ_CONSTRUCTOR, MSG_RESEND_REQ_CONSTRUCTOR,
        PING_CONSTRUCTOR, PING_DELAY_DISCONNECT_CONSTRUCTOR, PONG_CONSTRUCTOR, RPC_RESULT_CONSTRUCTOR,
    };
    use crate::tl::TlWriter;

    #[test]
    fn msgs_ack_parses_fixed_u_message_id_vector() {
        let first = 0x0102_0304_0506_0708_i64;
        let second = 0x1112_1314_1516_1718_i64;
        let encoded = build_msgs_ack(&[first, second]).expect("encode ack");
        assert_eq!(&encoded[0..4], &MSGS_ACK_CONSTRUCTOR.to_le_bytes());
        assert_eq!(parse_msgs_ack(&encoded).expect("parse ack"), vec![first, second]);
    }

    #[test]
    fn msgs_ack_rejects_trailing_bytes() {
        let mut encoded = build_msgs_ack(&[0x2122_2324_2526_2728_i64]).expect("encode ack");
        encoded.extend_from_slice(&0_i32.to_le_bytes());
        assert!(parse_msgs_ack(&encoded).is_err());
    }

    #[test]
    fn ping_delay_disconnect_serializes_fixed_u_fields_exactly() {
        let ping_id = 0x0102_0304_0506_0708_i64;
        let disconnect_delay = 60_i32;
        let encoded = build_ping_delay_disconnect(ping_id, disconnect_delay);
        assert_eq!(encoded.len(), 16);
        assert_eq!(&encoded[0..4], &PING_DELAY_DISCONNECT_CONSTRUCTOR.to_le_bytes());
        assert_eq!(&encoded[4..12], &ping_id.to_le_bytes());
        assert_eq!(&encoded[12..16], &disconnect_delay.to_le_bytes());
    }

    #[test]
    fn ping_delay_disconnect_preserves_signed_boundaries_and_exact_layout() {
        let encoded = build_ping_delay_disconnect(i64::MIN, i32::MIN);
        let mut expected = Vec::new();
        expected.extend_from_slice(&PING_DELAY_DISCONNECT_CONSTRUCTOR.to_le_bytes());
        expected.extend_from_slice(&i64::MIN.to_le_bytes());
        expected.extend_from_slice(&i32::MIN.to_le_bytes());
        assert_eq!(encoded, expected);

        let encoded = build_ping_delay_disconnect(i64::MAX, i32::MAX);
        let mut expected = Vec::new();
        expected.extend_from_slice(&PING_DELAY_DISCONNECT_CONSTRUCTOR.to_le_bytes());
        expected.extend_from_slice(&i64::MAX.to_le_bytes());
        expected.extend_from_slice(&i32::MAX.to_le_bytes());
        assert_eq!(encoded, expected);
    }

    #[test]
    fn req_pq_serializes_fixed_u_nonce_exactly() {
        let nonce = [
            0x00, 0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77,
            0x88, 0x99, 0xaa, 0xbb, 0xcc, 0xdd, 0xee, 0xff,
        ];
        let encoded = build_req_pq(&nonce);
        assert_eq!(encoded.len(), 20);
        assert_eq!(&encoded[0..4], &REQ_PQ_CONSTRUCTOR.to_le_bytes());
        assert_eq!(&encoded[4..20], &nonce);
    }

    #[test]
    fn req_pq_preserves_raw_int128_bytes_without_tl_bytes_framing() {
        for nonce in [[0_u8; 16], [0xff_u8; 16]] {
            let encoded = build_req_pq(&nonce);
            let mut expected = Vec::with_capacity(20);
            expected.extend_from_slice(&REQ_PQ_CONSTRUCTOR.to_le_bytes());
            expected.extend_from_slice(&nonce);
            assert_eq!(encoded, expected);
        }
    }

    #[test]
    fn destroy_auth_key_serializes_fixed_u_zero_field_request_exactly() {
        let encoded = build_destroy_auth_key();
        assert_eq!(encoded, DESTROY_AUTH_KEY_CONSTRUCTOR.to_le_bytes());
        assert_eq!(encoded.len(), 4);
    }

    #[test]
    fn destroy_session_serializes_fixed_u_request_exactly() {
        let session_id = 0x0102_0304_0506_0708_i64;
        let encoded = build_destroy_session(session_id);
        assert_eq!(encoded.len(), 12);
        assert_eq!(&encoded[0..4], &DESTROY_SESSION_CONSTRUCTOR.to_le_bytes());
        assert_eq!(&encoded[4..12], &session_id.to_le_bytes());
    }

    #[test]
    fn destroy_session_preserves_signed_boundaries_and_stays_separate_from_result_parser() {
        for session_id in [i64::MIN, i64::MAX] {
            let encoded = build_destroy_session(session_id);
            let mut expected = Vec::new();
            expected.extend_from_slice(&DESTROY_SESSION_CONSTRUCTOR.to_le_bytes());
            expected.extend_from_slice(&session_id.to_le_bytes());
            assert_eq!(encoded, expected);
            assert!(parse_destroy_session_result(&encoded).is_err());
        }
    }

    #[test]
    fn get_future_salts_serializes_fixed_u_request_exactly() {
        let num = 0x0102_0304_i32;
        let encoded = build_get_future_salts(num);
        assert_eq!(encoded.len(), 8);
        assert_eq!(&encoded[0..4], &GET_FUTURE_SALTS_CONSTRUCTOR.to_le_bytes());
        assert_eq!(&encoded[4..8], &num.to_le_bytes());
    }

    #[test]
    fn get_future_salts_preserves_signed_boundaries_and_stays_separate_from_response_parser() {
        for num in [i32::MIN, i32::MAX] {
            let encoded = build_get_future_salts(num);
            let mut expected = Vec::new();
            expected.extend_from_slice(&GET_FUTURE_SALTS_CONSTRUCTOR.to_le_bytes());
            expected.extend_from_slice(&num.to_le_bytes());
            assert_eq!(encoded, expected);
            assert!(parse_future_salts(&encoded).is_err());
        }
    }

    #[test]
    fn ping_serializes_fixed_u_request_exactly() {
        let ping_id = 0x0102_0304_0506_0708_i64;
        let encoded = build_ping(ping_id);
        assert_eq!(encoded.len(), 12);
        assert_eq!(&encoded[0..4], &PING_CONSTRUCTOR.to_le_bytes());
        assert_eq!(&encoded[4..12], &ping_id.to_le_bytes());
    }

    #[test]
    fn ping_preserves_signed_boundaries_and_stays_separate_from_pong_parser() {
        for ping_id in [i64::MIN, i64::MAX] {
            let encoded = build_ping(ping_id);
            let mut expected = Vec::new();
            expected.extend_from_slice(&PING_CONSTRUCTOR.to_le_bytes());
            expected.extend_from_slice(&ping_id.to_le_bytes());
            assert_eq!(encoded, expected);
            assert!(parse_pong(&encoded).is_err());
        }
    }

    #[test]
    fn pong_parses_fixed_u_message_and_ping_ids() {
        let message_id = 0x3132_3334_3536_3738_i64;
        let ping_id = 0x4142_4344_4546_4748_i64;
        let mut writer = TlWriter::new();
        writer.write_u32(PONG_CONSTRUCTOR);
        writer.write_i64(message_id);
        writer.write_i64(ping_id);
        let parsed = parse_pong(writer.as_slice()).expect("parse pong");
        assert_eq!(parsed.message_id, message_id);
        assert_eq!(parsed.ping_id, ping_id);
    }

    #[test]
    fn pong_rejects_trailing_bytes() {
        let mut writer = TlWriter::new();
        writer.write_u32(PONG_CONSTRUCTOR);
        writer.write_i64(100);
        writer.write_i64(200);
        writer.write_i32(0);
        assert!(parse_pong(writer.as_slice()).is_err());
    }

    #[test]
    fn rpc_result_parses_fixed_u_request_id_and_preserves_opaque_result() {
        let request_message_id = 0x5152_5354_5556_5758_i64;
        let result_constructor = 0x9972_75b5_u32;
        let mut writer = TlWriter::new();
        writer.write_u32(RPC_RESULT_CONSTRUCTOR);
        writer.write_i64(request_message_id);
        writer.write_u32(result_constructor);
        let parsed = parse_rpc_result(writer.as_slice()).expect("parse rpc_result");
        assert_eq!(parsed.request_message_id, request_message_id);
        assert_eq!(parsed.result_body, result_constructor.to_le_bytes());
    }

    #[test]
    fn rpc_result_rejects_missing_or_unaligned_result_object() {
        let mut writer = TlWriter::new();
        writer.write_u32(RPC_RESULT_CONSTRUCTOR);
        writer.write_i64(100);
        assert!(parse_rpc_result(writer.as_slice()).is_err());

        let mut unaligned = writer.into_bytes();
        unaligned.push(0xff);
        assert!(parse_rpc_result(&unaligned).is_err());
    }

    #[test]
    fn message_container_parses_fixed_u_entries_and_preserves_opaque_bodies() {
        let mut writer = TlWriter::new();
        writer.write_u32(MESSAGE_CONTAINER_CONSTRUCTOR);
        writer.write_i32(2);
        writer.write_i64(0x1112_1314_1516_1719_i64);
        writer.write_i32(1);
        writer.write_i32(4);
        writer.write_u32(0x9972_75b5);
        writer.write_i64(0x2122_2324_2526_2729_i64);
        writer.write_i32(2);
        writer.write_i32(8);
        writer.write_u32(0x3477_73c5);
        writer.write_u32(0x0102_0304);

        let parsed = parse_message_container(writer.as_slice()).expect("parse msg_container");
        assert_eq!(parsed.entries.len(), 2);
        assert_eq!(parsed.entries[0].message_id, 0x1112_1314_1516_1719_i64);
        assert_eq!(parsed.entries[0].sequence_number, 1);
        assert_eq!(parsed.entries[0].body, 0x9972_75b5_u32.to_le_bytes());
        assert_eq!(parsed.entries[1].sequence_number, 2);
        assert_eq!(parsed.entries[1].body.len(), 8);
    }

    #[test]
    fn rpc_drop_answer_request_serializes_fixed_u_request_exactly() {
        let req_msg_id = 0x0102_0304_0506_0708_i64;
        let encoded = build_rpc_drop_answer(req_msg_id);
        assert_eq!(encoded.len(), 12);
        assert_eq!(&encoded[0..4], &RPC_DROP_ANSWER_CONSTRUCTOR.to_le_bytes());
        assert_eq!(&encoded[4..12], &req_msg_id.to_le_bytes());
    }

    #[test]
    fn rpc_drop_answer_request_preserves_signed_boundaries_and_stays_separate_from_response_parser() {
        for req_msg_id in [i64::MIN, i64::MAX] {
            let encoded = build_rpc_drop_answer(req_msg_id);
            let mut expected = Vec::new();
            expected.extend_from_slice(&RPC_DROP_ANSWER_CONSTRUCTOR.to_le_bytes());
            expected.extend_from_slice(&req_msg_id.to_le_bytes());
            assert_eq!(encoded, expected);
            assert!(parse_rpc_drop_answer(&encoded).is_err());
        }
    }

    #[test]
    fn rpc_drop_answer_parses_all_fixed_u_variants() {
        assert_eq!(
            parse_rpc_drop_answer(&RPC_ANSWER_UNKNOWN_CONSTRUCTOR.to_le_bytes())
                .expect("parse rpc_answer_unknown"),
            RpcDropAnswer::Unknown
        );
        assert_eq!(
            parse_rpc_drop_answer(&RPC_ANSWER_DROPPED_RUNNING_CONSTRUCTOR.to_le_bytes())
                .expect("parse rpc_answer_dropped_running"),
            RpcDropAnswer::DroppedRunning
        );

        let message_id = 0x1112_1314_1516_1718_i64;
        let mut dropped = TlWriter::new();
        dropped.write_u32(RPC_ANSWER_DROPPED_CONSTRUCTOR);
        dropped.write_i64(message_id);
        dropped.write_i32(7);
        dropped.write_i32(4096);
        assert_eq!(
            parse_rpc_drop_answer(dropped.as_slice()).expect("parse rpc_answer_dropped"),
            RpcDropAnswer::Dropped {
                message_id,
                sequence_number: 7,
                bytes: 4096,
            }
        );
    }

    #[test]
    fn rpc_drop_answer_rejects_unknown_constructor_truncation_and_trailing_bytes() {
        assert!(parse_rpc_drop_answer(&0_u32.to_le_bytes()).is_err());
        let unknown = RPC_ANSWER_UNKNOWN_CONSTRUCTOR.to_le_bytes();
        assert!(parse_rpc_drop_answer(&unknown[..3]).is_err());

        let mut unknown_trailing = unknown.to_vec();
        unknown_trailing.extend_from_slice(&0_u32.to_le_bytes());
        assert!(parse_rpc_drop_answer(&unknown_trailing).is_err());

        let mut running_trailing = RPC_ANSWER_DROPPED_RUNNING_CONSTRUCTOR.to_le_bytes().to_vec();
        running_trailing.extend_from_slice(&0_u32.to_le_bytes());
        assert!(parse_rpc_drop_answer(&running_trailing).is_err());

        let mut dropped = TlWriter::new();
        dropped.write_u32(RPC_ANSWER_DROPPED_CONSTRUCTOR);
        dropped.write_i64(0x2122_2324_2526_2728_i64);
        dropped.write_i32(3);
        dropped.write_i32(64);
        let exact = dropped.into_bytes();
        for cut in [4_usize, 8, 12, 16, exact.len() - 1] {
            assert!(parse_rpc_drop_answer(&exact[..cut]).is_err());
        }
        let mut dropped_trailing = exact;
        dropped_trailing.extend_from_slice(&0_u32.to_le_bytes());
        assert!(parse_rpc_drop_answer(&dropped_trailing).is_err());
    }

    #[test]
    fn destroy_session_result_parses_both_fixed_u_variants() {
        let ok_session_id = 0x3132_3334_3536_3738_i64;
        let mut ok = TlWriter::new();
        ok.write_u32(DESTROY_SESSION_OK_CONSTRUCTOR);
        ok.write_i64(ok_session_id);
        assert_eq!(
            parse_destroy_session_result(ok.as_slice()).expect("parse destroy_session_ok"),
            DestroySessionResult::Ok {
                session_id: ok_session_id,
            }
        );

        let none_session_id = 0x4142_4344_4546_4748_i64;
        let mut none = TlWriter::new();
        none.write_u32(DESTROY_SESSION_NONE_CONSTRUCTOR);
        none.write_i64(none_session_id);
        assert_eq!(
            parse_destroy_session_result(none.as_slice()).expect("parse destroy_session_none"),
            DestroySessionResult::None {
                session_id: none_session_id,
            }
        );
    }

    #[test]
    fn destroy_session_result_rejects_unknown_constructor_truncation_and_trailing_bytes() {
        let mut unknown = TlWriter::new();
        unknown.write_u32(0);
        unknown.write_i64(1);
        assert!(parse_destroy_session_result(unknown.as_slice()).is_err());

        let mut exact = TlWriter::new();
        exact.write_u32(DESTROY_SESSION_OK_CONSTRUCTOR);
        exact.write_i64(0x5152_5354_5556_5758_i64);
        let exact = exact.into_bytes();
        for cut in [0_usize, 1, 3, 4, 8, exact.len() - 1] {
            assert!(parse_destroy_session_result(&exact[..cut]).is_err());
        }

        let mut trailing = exact;
        trailing.extend_from_slice(&0_u32.to_le_bytes());
        assert!(parse_destroy_session_result(&trailing).is_err());
    }

    #[test]
    fn http_wait_parses_all_fixed_u_fields_exactly() {
        let mut writer = TlWriter::new();
        writer.write_u32(HTTP_WAIT_CONSTRUCTOR);
        writer.write_i32(100);
        writer.write_i32(30);
        writer.write_i32(25_000);
        let parsed = parse_http_wait(writer.as_slice()).expect("parse http_wait");
        assert_eq!(parsed.max_delay, 100);
        assert_eq!(parsed.wait_after, 30);
        assert_eq!(parsed.max_wait, 25_000);
    }

    #[test]
    fn http_wait_rejects_wrong_constructor_truncation_and_trailing_bytes() {
        let mut wrong = TlWriter::new();
        wrong.write_u32(0);
        wrong.write_i32(100);
        wrong.write_i32(30);
        wrong.write_i32(25_000);
        assert!(parse_http_wait(wrong.as_slice()).is_err());

        let mut exact = TlWriter::new();
        exact.write_u32(HTTP_WAIT_CONSTRUCTOR);
        exact.write_i32(-1);
        exact.write_i32(i32::MIN);
        exact.write_i32(i32::MAX);
        let exact = exact.into_bytes();
        for cut in 0..exact.len() {
            assert!(parse_http_wait(&exact[..cut]).is_err());
        }

        let mut trailing = exact;
        trailing.extend_from_slice(&0_u32.to_le_bytes());
        assert!(parse_http_wait(&trailing).is_err());
    }

    fn ip_port_plain_fixture() -> Vec<u8> {
        let mut writer = TlWriter::new();
        writer.write_u32(IP_PORT_CONSTRUCTOR);
        writer.write_i32(i32::MIN);
        writer.write_i32(-443);
        writer.into_bytes()
    }

    fn ip_port_secret_fixture() -> Vec<u8> {
        let mut writer = TlWriter::new();
        writer.write_u32(IP_PORT_SECRET_CONSTRUCTOR);
        writer.write_i32(i32::MAX);
        writer.write_i32(0);
        writer.write_bytes(&[0x01, 0x02, 0x03, 0xff]).expect("write secret");
        writer.into_bytes()
    }

    fn access_point_rule_fixture() -> Vec<u8> {
        let mut writer = TlWriter::new();
        writer.write_u32(ACCESS_POINT_RULE_CONSTRUCTOR);
        writer.write_string("+1,-1204").expect("write phone rules");
        writer.write_i32(-2);
        writer.write_vector_length(2).expect("write ips vector");
        writer.write_u32(IP_PORT_CONSTRUCTOR);
        writer.write_i32(0x0102_0304);
        writer.write_i32(443);
        writer.write_u32(IP_PORT_SECRET_CONSTRUCTOR);
        writer.write_i32(-1);
        writer.write_i32(80);
        writer.write_bytes(&[0xaa, 0xbb, 0xcc]).expect("write secret");
        writer.into_bytes()
    }

    fn help_config_simple_fixture() -> Vec<u8> {
        let mut writer = TlWriter::new();
        writer.write_u32(HELP_CONFIG_SIMPLE_CONSTRUCTOR);
        writer.write_i32(-1);
        writer.write_i32(i32::MAX);
        writer.write_vector_length(1).expect("write rules vector");
        writer.write_u32(ACCESS_POINT_RULE_CONSTRUCTOR);
        writer.write_string("*").expect("write phone rules");
        writer.write_i32(5);
        writer.write_vector_length(2).expect("write ips vector");
        writer.write_u32(IP_PORT_CONSTRUCTOR);
        writer.write_i32(0x7f00_0001);
        writer.write_i32(443);
        writer.write_u32(IP_PORT_SECRET_CONSTRUCTOR);
        writer.write_i32(0x0102_0304);
        writer.write_i32(-1);
        writer.write_bytes(&[]).expect("write empty secret");
        writer.into_bytes()
    }

    #[test]
    fn ip_port_parses_both_fixed_u_variants_exactly() {
        assert_eq!(
            parse_ip_port(&ip_port_plain_fixture()).expect("parse ipPort"),
            IpPort::Plain {
                ipv4: i32::MIN,
                port: -443,
            }
        );
        assert_eq!(
            parse_ip_port(&ip_port_secret_fixture()).expect("parse ipPortSecret"),
            IpPort::Secret {
                ipv4: i32::MAX,
                port: 0,
                secret: vec![0x01, 0x02, 0x03, 0xff],
            }
        );
    }

    #[test]
    fn ip_port_rejects_wrong_constructor_truncation_and_trailing_bytes() {
        let exact = ip_port_secret_fixture();
        let mut wrong = exact.clone();
        wrong[..4].copy_from_slice(&0_u32.to_le_bytes());
        assert!(parse_ip_port(&wrong).is_err());
        for cut in 0..exact.len() {
            assert!(parse_ip_port(&exact[..cut]).is_err());
        }
        let mut trailing = exact;
        trailing.extend_from_slice(&0_u32.to_le_bytes());
        assert!(parse_ip_port(&trailing).is_err());
    }

    #[test]
    fn access_point_rule_parses_nested_ip_ports_exactly() {
        let parsed = parse_access_point_rule(&access_point_rule_fixture()).expect("parse rule");
        assert_eq!(
            parsed,
            AccessPointRule {
                phone_prefix_rules: "+1,-1204".to_string(),
                dc_id: -2,
                ips: vec![
                    IpPort::Plain {
                        ipv4: 0x0102_0304,
                        port: 443,
                    },
                    IpPort::Secret {
                        ipv4: -1,
                        port: 80,
                        secret: vec![0xaa, 0xbb, 0xcc],
                    },
                ],
            }
        );
    }

    #[test]
    fn access_point_rule_rejects_malformed_nested_input_and_trailing_bytes() {
        let exact = access_point_rule_fixture();
        for cut in 0..exact.len() {
            assert!(parse_access_point_rule(&exact[..cut]).is_err());
        }

        let mut wrong_rule = exact.clone();
        wrong_rule[..4].copy_from_slice(&0_u32.to_le_bytes());
        assert!(parse_access_point_rule(&wrong_rule).is_err());

        let mut wrong_vector = TlWriter::new();
        wrong_vector.write_u32(ACCESS_POINT_RULE_CONSTRUCTOR);
        wrong_vector.write_string("+1").expect("write phone rules");
        wrong_vector.write_i32(2);
        wrong_vector.write_u32(0);
        wrong_vector.write_i32(0);
        assert!(parse_access_point_rule(wrong_vector.as_slice()).is_err());

        let mut invalid_utf8 = Vec::new();
        invalid_utf8.extend_from_slice(&ACCESS_POINT_RULE_CONSTRUCTOR.to_le_bytes());
        invalid_utf8.extend_from_slice(&[1, 0xff, 0, 0]);
        invalid_utf8.extend_from_slice(&2_i32.to_le_bytes());
        invalid_utf8.extend_from_slice(&0x1cb5_c415_u32.to_le_bytes());
        invalid_utf8.extend_from_slice(&0_i32.to_le_bytes());
        assert!(parse_access_point_rule(&invalid_utf8).is_err());

        let mut wrong_ip = exact.clone();
        let first_ip_constructor = wrong_ip
            .windows(4)
            .position(|bytes| bytes == IP_PORT_CONSTRUCTOR.to_le_bytes())
            .expect("first nested ip");
        wrong_ip[first_ip_constructor..first_ip_constructor + 4]
            .copy_from_slice(&0_u32.to_le_bytes());
        assert!(parse_access_point_rule(&wrong_ip).is_err());

        let mut trailing = exact;
        trailing.extend_from_slice(&0_u32.to_le_bytes());
        assert!(parse_access_point_rule(&trailing).is_err());
    }

    #[test]
    fn help_config_simple_parses_fixed_u_nested_rules_exactly() {
        let parsed =
            parse_help_config_simple(&help_config_simple_fixture()).expect("parse configSimple");
        assert_eq!(parsed.date, -1);
        assert_eq!(parsed.expires, i32::MAX);
        assert_eq!(parsed.rules.len(), 1);
        assert_eq!(parsed.rules[0].phone_prefix_rules, "*");
        assert_eq!(parsed.rules[0].dc_id, 5);
        assert_eq!(
            parsed.rules[0].ips,
            vec![
                IpPort::Plain {
                    ipv4: 0x7f00_0001,
                    port: 443,
                },
                IpPort::Secret {
                    ipv4: 0x0102_0304,
                    port: -1,
                    secret: Vec::new(),
                },
            ]
        );
    }

    #[test]
    fn help_config_simple_rejects_wrong_nested_constructor_truncation_and_trailing_bytes() {
        let exact = help_config_simple_fixture();
        for cut in 0..exact.len() {
            assert!(parse_help_config_simple(&exact[..cut]).is_err());
        }

        let mut wrong_config = exact.clone();
        wrong_config[..4].copy_from_slice(&0_u32.to_le_bytes());
        assert!(parse_help_config_simple(&wrong_config).is_err());

        let mut wrong_rules_vector = TlWriter::new();
        wrong_rules_vector.write_u32(HELP_CONFIG_SIMPLE_CONSTRUCTOR);
        wrong_rules_vector.write_i32(1);
        wrong_rules_vector.write_i32(2);
        wrong_rules_vector.write_u32(0);
        wrong_rules_vector.write_i32(0);
        assert!(parse_help_config_simple(wrong_rules_vector.as_slice()).is_err());

        let mut wrong_rule = exact.clone();
        let rule_constructor = wrong_rule
            .windows(4)
            .position(|bytes| bytes == ACCESS_POINT_RULE_CONSTRUCTOR.to_le_bytes())
            .expect("nested rule");
        wrong_rule[rule_constructor..rule_constructor + 4]
            .copy_from_slice(&0_u32.to_le_bytes());
        assert!(parse_help_config_simple(&wrong_rule).is_err());

        let mut trailing = exact;
        trailing.extend_from_slice(&0_u32.to_le_bytes());
        assert!(parse_help_config_simple(&trailing).is_err());
    }

    #[test]
    fn message_container_rejects_invalid_lengths_counts_and_trailing_bytes() {
        let mut too_short = TlWriter::new();
        too_short.write_u32(MESSAGE_CONTAINER_CONSTRUCTOR);
        too_short.write_i32(1);
        too_short.write_i64(0x1112_1314_1516_1719_i64);
        too_short.write_i32(1);
        too_short.write_i32(0);
        assert!(parse_message_container(too_short.as_slice()).is_err());

        let mut unaligned = TlWriter::new();
        unaligned.write_u32(MESSAGE_CONTAINER_CONSTRUCTOR);
        unaligned.write_i32(1);
        unaligned.write_i64(0x1112_1314_1516_1719_i64);
        unaligned.write_i32(1);
        unaligned.write_i32(5);
        unaligned.write_u32(0x9972_75b5);
        let mut unaligned_bytes = unaligned.into_bytes();
        unaligned_bytes.push(0xff);
        assert!(parse_message_container(&unaligned_bytes).is_err());

        let mut too_many = TlWriter::new();
        too_many.write_u32(MESSAGE_CONTAINER_CONSTRUCTOR);
        too_many.write_i32(1025);
        assert!(parse_message_container(too_many.as_slice()).is_err());

        let mut trailing = TlWriter::new();
        trailing.write_u32(MESSAGE_CONTAINER_CONSTRUCTOR);
        trailing.write_i32(0);
        trailing.write_u32(0);
        assert!(parse_message_container(trailing.as_slice()).is_err());
    }

    #[test]
    fn msg_resend_req_encodes_fixed_u_constructor_and_single_answer_id() {
        let answer_id = 0x0102_0304_0506_0708_i64;
        let encoded = build_msg_resend_req(&[answer_id]).expect("encode resend request");
        assert_eq!(&encoded[0..4], &MSG_RESEND_REQ_CONSTRUCTOR.to_le_bytes());
        assert_eq!(&encoded[4..8], &0x1cb5_c415_u32.to_le_bytes());
        assert_eq!(&encoded[8..12], &1_i32.to_le_bytes());
        assert_eq!(&encoded[12..20], &answer_id.to_le_bytes());
    }

    #[test]
    fn msgs_state_req_encodes_fixed_u_constructor_and_current_request_id() {
        let request_id = 0x1112_1314_1516_1718_i64;
        let encoded = build_msgs_state_req(&[request_id]).expect("encode state request");
        assert_eq!(&encoded[0..4], &MSGS_STATE_REQ_CONSTRUCTOR.to_le_bytes());
        assert_eq!(&encoded[4..8], &0x1cb5_c415_u32.to_le_bytes());
        assert_eq!(&encoded[8..12], &1_i32.to_le_bytes());
        assert_eq!(&encoded[12..20], &request_id.to_le_bytes());
    }

    #[test]
    fn msgs_state_info_parses_binary_state_bytes() {
        let query_id = 0x2122_2324_2526_2728_i64;
        let mut writer = TlWriter::new();
        writer.write_u32(MSGS_STATE_INFO_CONSTRUCTOR);
        writer.write_i64(query_id);
        writer.write_bytes(&[0x04]).expect("write state byte");

        let parsed = parse_msgs_state_info(writer.as_slice()).expect("parse state info");
        assert_eq!(parsed.request_message_id, query_id);
        assert_eq!(parsed.states, vec![0x04]);
    }

    #[test]
    fn msgs_all_info_requires_one_state_byte_per_message_id() {
        let first = 0x3132_3334_3536_3738_i64;
        let second = 0x4142_4344_4546_4748_i64;
        let mut writer = TlWriter::new();
        writer.write_u32(MSGS_ALL_INFO_CONSTRUCTOR);
        writer.write_vector_length(2).expect("write ids vector");
        writer.write_i64(first);
        writer.write_i64(second);
        writer.write_bytes(&[0x04, 0x01]).expect("write state bytes");

        let parsed = parse_msgs_all_info(writer.as_slice()).expect("parse all info");
        assert_eq!(parsed.message_ids, vec![first, second]);
        assert_eq!(parsed.states, vec![0x04, 0x01]);
    }

    #[test]
    fn msg_detailed_info_parses_all_fixed_u_fields() {
        let message_id = 0x3132_3334_3536_3738_i64;
        let answer_message_id = 0x4142_4344_4546_4748_i64;
        let mut writer = TlWriter::new();
        writer.write_u32(MSG_DETAILED_INFO_CONSTRUCTOR);
        writer.write_i64(message_id);
        writer.write_i64(answer_message_id);
        writer.write_i32(8192);
        writer.write_i32(5);

        let parsed = parse_msg_detailed_info(writer.as_slice()).expect("parse detailed info");
        assert_eq!(parsed.message_id, message_id);
        assert_eq!(parsed.answer_message_id, answer_message_id);
        assert_eq!(parsed.bytes, 8192);
        assert_eq!(parsed.status, 5);
    }

    #[test]
    fn msg_detailed_info_rejects_wrong_constructor_truncation_and_trailing_bytes() {
        let mut writer = TlWriter::new();
        writer.write_u32(MSG_DETAILED_INFO_CONSTRUCTOR);
        writer.write_i64(0x5152_5354_5556_5758_i64);
        writer.write_i64(0x6162_6364_6566_6768_i64);
        writer.write_i32(32);
        writer.write_i32(7);
        let exact = writer.into_bytes();

        let mut wrong_constructor = exact.clone();
        wrong_constructor[..4].copy_from_slice(&0_u32.to_le_bytes());
        assert!(parse_msg_detailed_info(&wrong_constructor).is_err());

        for cut in [1_usize, 4, 8, 12, 20, exact.len() - 1] {
            assert!(parse_msg_detailed_info(&exact[..cut]).is_err());
        }

        let mut trailing = exact;
        trailing.extend_from_slice(&0_u32.to_le_bytes());
        assert!(parse_msg_detailed_info(&trailing).is_err());
    }

    #[test]
    fn msg_new_detailed_info_parses_all_fixed_u_fields() {
        let answer_message_id = 0x5152_5354_5556_5759_i64;
        let mut writer = TlWriter::new();
        writer.write_u32(MSG_NEW_DETAILED_INFO_CONSTRUCTOR);
        writer.write_i64(answer_message_id);
        writer.write_i32(4096);
        writer.write_i32(3);

        let parsed =
            parse_msg_new_detailed_info(writer.as_slice()).expect("parse new detailed info");
        assert_eq!(parsed.answer_message_id, answer_message_id);
        assert_eq!(parsed.bytes, 4096);
        assert_eq!(parsed.status, 3);
    }

    #[test]
    fn msg_new_detailed_info_rejects_truncation_and_trailing_bytes() {
        let mut writer = TlWriter::new();
        writer.write_u32(MSG_NEW_DETAILED_INFO_CONSTRUCTOR);
        writer.write_i64(0x6162_6364_6566_6769_i64);
        writer.write_i32(4);
        writer.write_i32(1);
        let exact = writer.into_bytes();

        assert!(parse_msg_new_detailed_info(&exact[..exact.len() - 4]).is_err());

        let mut trailing = exact;
        trailing.extend_from_slice(&0_u32.to_le_bytes());
        assert!(parse_msg_new_detailed_info(&trailing).is_err());
    }

    fn bad_msg_notification_fixture() -> Vec<u8> {
        let mut writer = TlWriter::new();
        writer.write_u32(BAD_MSG_NOTIFICATION_CONSTRUCTOR);
        writer.write_i64(0x0102_0304_0506_0708_i64);
        writer.write_i32(9);
        writer.write_i32(16);
        writer.into_bytes()
    }

    #[test]
    fn bad_msg_notification_parses_all_fixed_u_fields() {
        let parsed = parse_bad_msg_notification(&bad_msg_notification_fixture())
            .expect("parse bad_msg_notification");
        assert_eq!(parsed.bad_message_id, 0x0102_0304_0506_0708_i64);
        assert_eq!(parsed.bad_message_sequence_number, 9);
        assert_eq!(parsed.error_code, 16);
    }

    #[test]
    fn bad_msg_notification_rejects_wrong_constructor_truncation_and_trailing_bytes() {
        let exact = bad_msg_notification_fixture();

        let mut wrong_constructor = exact.clone();
        wrong_constructor[..4].copy_from_slice(&0_u32.to_le_bytes());
        assert!(parse_bad_msg_notification(&wrong_constructor).is_err());

        for cut in [1_usize, 4, 8, 12, 16, exact.len() - 1] {
            assert!(parse_bad_msg_notification(&exact[..cut]).is_err());
        }

        let mut trailing = exact;
        trailing.extend_from_slice(&0_u32.to_le_bytes());
        assert!(parse_bad_msg_notification(&trailing).is_err());
    }

    fn bad_server_salt_fixture() -> Vec<u8> {
        let mut writer = TlWriter::new();
        writer.write_u32(BAD_SERVER_SALT_CONSTRUCTOR);
        writer.write_i64(0x1112_1314_1516_1718_i64);
        writer.write_i32(7);
        writer.write_i32(48);
        writer.write_i64(0x2122_2324_2526_2728_i64);
        writer.into_bytes()
    }

    #[test]
    fn bad_server_salt_parses_all_fixed_u_fields() {
        let parsed =
            parse_bad_server_salt(&bad_server_salt_fixture()).expect("parse bad_server_salt");
        assert_eq!(parsed.bad_message_id, 0x1112_1314_1516_1718_i64);
        assert_eq!(parsed.bad_message_sequence_number, 7);
        assert_eq!(parsed.error_code, 48);
        assert_eq!(parsed.new_server_salt, 0x2122_2324_2526_2728_i64);
    }

    #[test]
    fn bad_server_salt_rejects_wrong_constructor_truncation_and_trailing_bytes() {
        let exact = bad_server_salt_fixture();

        let mut wrong_constructor = exact.clone();
        wrong_constructor[..4].copy_from_slice(&0_u32.to_le_bytes());
        assert!(parse_bad_server_salt(&wrong_constructor).is_err());

        for cut in [1_usize, 4, 8, 12, 16, 20, 24, exact.len() - 1] {
            assert!(parse_bad_server_salt(&exact[..cut]).is_err());
        }

        let mut trailing = exact;
        trailing.extend_from_slice(&0_u32.to_le_bytes());
        assert!(parse_bad_server_salt(&trailing).is_err());
    }

    fn future_salts_fixture() -> Vec<u8> {
        let mut writer = TlWriter::new();
        writer.write_u32(FUTURE_SALTS_CONSTRUCTOR);
        writer.write_i64(0x1112_1314_1516_1718_i64);
        writer.write_i32(1_700_000_000);
        writer.write_vector_length(2).expect("write salts vector");
        writer.write_u32(FUTURE_SALT_CONSTRUCTOR);
        writer.write_i32(1_700_000_100);
        writer.write_i32(1_700_000_200);
        writer.write_i64(0x2122_2324_2526_2728_i64);
        writer.write_u32(FUTURE_SALT_CONSTRUCTOR);
        writer.write_i32(1_700_000_300);
        writer.write_i32(1_700_000_400);
        writer.write_i64(0x3132_3334_3536_3738_i64);
        writer.into_bytes()
    }

    #[test]
    fn future_salts_parses_all_fixed_u_fields() {
        let parsed = parse_future_salts(&future_salts_fixture()).expect("parse future salts");
        assert_eq!(parsed.request_message_id, 0x1112_1314_1516_1718_i64);
        assert_eq!(parsed.now, 1_700_000_000);
        assert_eq!(parsed.salts.len(), 2);
        assert_eq!(parsed.salts[0].valid_since, 1_700_000_100);
        assert_eq!(parsed.salts[0].valid_until, 1_700_000_200);
        assert_eq!(parsed.salts[0].salt, 0x2122_2324_2526_2728_i64);
        assert_eq!(parsed.salts[1].valid_since, 1_700_000_300);
        assert_eq!(parsed.salts[1].valid_until, 1_700_000_400);
        assert_eq!(parsed.salts[1].salt, 0x3132_3334_3536_3738_i64);
    }

    #[test]
    fn future_salts_rejects_wrong_constructors_truncation_impossible_count_and_trailing() {
        let exact = future_salts_fixture();

        let mut wrong_top = exact.clone();
        wrong_top[..4].copy_from_slice(&0_u32.to_le_bytes());
        assert!(parse_future_salts(&wrong_top).is_err());

        let mut wrong_vector = exact.clone();
        wrong_vector[16..20].copy_from_slice(&0_u32.to_le_bytes());
        assert!(parse_future_salts(&wrong_vector).is_err());

        let mut wrong_nested = exact.clone();
        wrong_nested[24..28].copy_from_slice(&0_u32.to_le_bytes());
        assert!(parse_future_salts(&wrong_nested).is_err());

        for cut in [1_usize, 4, 8, 12, 16, 20, 24, 28, exact.len() - 1] {
            assert!(parse_future_salts(&exact[..cut]).is_err());
        }

        let mut impossible_count = exact.clone();
        impossible_count[20..24].copy_from_slice(&3_i32.to_le_bytes());
        assert!(parse_future_salts(&impossible_count).is_err());

        let mut trailing = exact;
        trailing.extend_from_slice(&0_u32.to_le_bytes());
        assert!(parse_future_salts(&trailing).is_err());
    }
}
