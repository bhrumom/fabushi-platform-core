//! Protocol-facing state machines and schema coverage checks.
//!
//! Network transports and generated TL constructors will be added behind this
//! boundary. The current API makes authorization transitions and upstream drift
//! independently testable before a specific transport is selected.

pub mod api;
pub mod auth;
pub mod crypto;
pub mod dc;
pub mod generated;
pub mod handshake;
pub mod request;
pub mod schema;
pub mod session;
pub mod srp;
pub mod tl;
pub mod tls;
pub mod transport;
pub mod updates;
pub mod web_proxy;

pub use api::{
    build_account_get_password, build_account_send_verify_email_code, build_account_verify_email,
    build_auth_cancel_code, build_auth_check_password, build_auth_check_recovery_password,
    build_auth_request_password_recovery, build_auth_resend_code, build_auth_send_code,
    build_auth_sign_in, build_auth_sign_up, build_destroy_auth_key, build_destroy_session, build_get_future_salts,
    build_init_connection_get_config, build_msg_resend_req, build_ping, build_ping_delay_disconnect,
    build_req_pq, build_req_pq_multi, build_rpc_drop_answer,
    build_msgs_ack, build_msgs_state_req, parse_bad_msg_notification, parse_bad_server_salt,
    parse_future_salts,
    parse_message_container, parse_msg_detailed_info, parse_msg_new_detailed_info,
    parse_account_email_verified, parse_account_password_prefix, parse_account_sent_email_code,
    parse_auth_authorization_user_id, parse_auth_password_recovery, parse_auth_sent_code, parse_bool,
    parse_access_point_rule, parse_config_dc_directory_prefix, parse_destroy_auth_key_result,
    parse_destroy_session_result, parse_help_config_simple, parse_http_wait, parse_ip_port, parse_msgs_ack,
    parse_msgs_all_info, parse_msgs_state_info, parse_pong, parse_rpc_drop_answer, parse_rpc_result,
    try_parse_rpc_error, AccessPointRule, AccountPasswordState, ApiRequestError, BadMsgNotification,
    BadServerSalt, ConfigDcDirectory, DestroyAuthKeyResult, DestroySessionResult, EmailVerifiedResult,
    FutureSalt, FutureSalts, HelpConfigSimple, HttpWait, InitConnection, IpPort, MessageContainer,
    MessageContainerEntry, MsgDetailedInfo,
    MsgNewDetailedInfo, MsgsAllInfo, MsgsStateInfo,
    NextCodeType,
    PasswordRecovery, Pong, RpcDropAnswer, RpcErrorResponse, RpcResultEnvelope, SentCode,
    SentCodeDelivery, SentCodeResult, SentEmailCode,
    ACCOUNT_EMAIL_VERIFIED_CONSTRUCTOR, ACCOUNT_EMAIL_VERIFIED_LOGIN_CONSTRUCTOR,
    ACCOUNT_GET_PASSWORD_CONSTRUCTOR, ACCOUNT_PASSWORD_CONSTRUCTOR,
    ACCOUNT_SENT_EMAIL_CODE_CONSTRUCTOR, ACCOUNT_SEND_VERIFY_EMAIL_CODE_CONSTRUCTOR,
    ACCOUNT_VERIFY_EMAIL_CONSTRUCTOR, AUTH_CANCEL_CODE_CONSTRUCTOR,
    AUTH_CHECK_PASSWORD_CONSTRUCTOR, AUTH_CHECK_RECOVERY_PASSWORD_CONSTRUCTOR,
    AUTH_PASSWORD_RECOVERY_CONSTRUCTOR, AUTH_REQUEST_PASSWORD_RECOVERY_CONSTRUCTOR,
    AUTH_RESEND_CODE_CONSTRUCTOR, AUTH_SEND_CODE_CONSTRUCTOR, AUTH_SENT_CODE_CONSTRUCTOR,
    AUTH_AUTHORIZATION_CONSTRUCTOR, AUTH_AUTHORIZATION_SIGN_UP_REQUIRED_CONSTRUCTOR,
    AUTH_SENT_CODE_PAYMENT_REQUIRED_CONSTRUCTOR, AUTH_SENT_CODE_SUCCESS_CONSTRUCTOR,
    AUTH_SIGN_IN_CONSTRUCTOR, AUTH_SIGN_UP_CONSTRUCTOR, CODE_SETTINGS_CONSTRUCTOR,
    ACCESS_POINT_RULE_CONSTRUCTOR, BAD_MSG_NOTIFICATION_CONSTRUCTOR, CONFIG_CONSTRUCTOR,
    DC_OPTION_CONSTRUCTOR, DESTROY_AUTH_KEY_CONSTRUCTOR, DESTROY_AUTH_KEY_FAIL_CONSTRUCTOR,
    DESTROY_AUTH_KEY_NONE_CONSTRUCTOR, DESTROY_AUTH_KEY_OK_CONSTRUCTOR, DESTROY_SESSION_CONSTRUCTOR,
    DESTROY_SESSION_NONE_CONSTRUCTOR, DESTROY_SESSION_OK_CONSTRUCTOR,
    EMAIL_VERIFICATION_CODE_CONSTRUCTOR,
    EMAIL_VERIFY_PURPOSE_LOGIN_SETUP_CONSTRUCTOR, HELP_GET_CONFIG_CONSTRUCTOR,
    BAD_SERVER_SALT_CONSTRUCTOR, FUTURE_SALTS_CONSTRUCTOR, FUTURE_SALT_CONSTRUCTOR,
    GET_FUTURE_SALTS_CONSTRUCTOR,
    HELP_CONFIG_SIMPLE_CONSTRUCTOR, HTTP_WAIT_CONSTRUCTOR, INIT_CONNECTION_CONSTRUCTOR,
    IP_PORT_CONSTRUCTOR, IP_PORT_SECRET_CONSTRUCTOR,
    INVOKE_WITH_LAYER_CONSTRUCTOR, MAX_MESSAGE_CONTAINER_ENTRIES, MESSAGE_CONTAINER_CONSTRUCTOR,
    MSG_DETAILED_INFO_CONSTRUCTOR,
    MSG_NEW_DETAILED_INFO_CONSTRUCTOR,
    MSG_RESEND_REQ_CONSTRUCTOR, MSGS_ACK_CONSTRUCTOR, MSGS_ALL_INFO_CONSTRUCTOR,
    MSGS_STATE_INFO_CONSTRUCTOR,
    MSGS_STATE_REQ_CONSTRUCTOR, MTPROTO_LAYER, PASSWORD_KDF_ALGO_CONSTRUCTOR,
    PING_CONSTRUCTOR, PING_DELAY_DISCONNECT_CONSTRUCTOR, PONG_CONSTRUCTOR, REQ_PQ_CONSTRUCTOR,
    REQ_PQ_MULTI_CONSTRUCTOR,
    RPC_ANSWER_DROPPED_CONSTRUCTOR, RPC_ANSWER_DROPPED_RUNNING_CONSTRUCTOR,
    RPC_ANSWER_UNKNOWN_CONSTRUCTOR, RPC_DROP_ANSWER_CONSTRUCTOR, RPC_ERROR_CONSTRUCTOR,
    RPC_RESULT_CONSTRUCTOR,
    USER_CONSTRUCTOR, USER_EMPTY_CONSTRUCTOR,
};
pub use auth::{
    AuthCommand, AuthError, AuthEvent, AuthorizationMachine, AuthorizationState, CodeDeliveryType,
};
pub use crypto::{
    aes_ige_decrypt, aes_ige_encrypt, decrypt_message, derive_aes_key_iv, encrypt_message,
    quick_ack_token, AuthKey, CryptoDirection, CryptoError, EncryptedEnvelope, PlainMessage,
};
pub use dc::{
    classify_rpc_error, derive_protocol_dc_identity, parse_migration, DcDirectory, DcEndpoint,
    DcError, DcPurpose, DcRoute, MigrationDirective, MigrationKind, ProtocolDcIdentity,
    ProtocolDcRole, RpcErrorAction,
};
pub use generated::{
    wire_constructor_by_name, wire_constructors_by_id, WireConstructor, WireSchema,
    WIRE_CONSTRUCTORS,
};
pub use handshake::{
    build_client_dh_inner_data, build_p_q_inner_data_dc, build_p_q_inner_data_temp_dc,
    build_req_dh_params, build_set_client_dh_params, decrypt_server_dh_inner_data,
    derive_tmp_aes_key_iv, factor_res_pq, parse_dh_gen_result, parse_res_pq,
    parse_server_dh_params, prepare_client_dh, rsa_pad, rsa_pad_with_random,
    select_server_key_fingerprint, telegram_server_rsa_key, validate_server_dh_parameters,
    AuthKeyHandshake, AuthKeyHandshakeState, DhGenAction, DhGenResult, EstablishedAuthKey,
    FactoredPq, HandshakeError, Nonce, PlaintextEnvelope, PreparedClientDh, ResPq, RsaPublicKey,
    ServerDhInnerData, ServerDhParams, CLIENT_DH_INNER_DATA_CONSTRUCTOR,
    DEFAULT_MAX_PLAINTEXT_BODY, DH_GEN_FAIL_CONSTRUCTOR, DH_GEN_OK_CONSTRUCTOR,
    DH_GEN_RETRY_CONSTRUCTOR, KNOWN_DH_PRIME_HEX, P_Q_INNER_DATA_DC_CONSTRUCTOR,
    P_Q_INNER_DATA_TEMP_DC_CONSTRUCTOR, REQ_DH_PARAMS_CONSTRUCTOR, RES_PQ_CONSTRUCTOR,
    SERVER_DH_INNER_DATA_CONSTRUCTOR, SERVER_DH_PARAMS_FAIL_CONSTRUCTOR,
    SERVER_DH_PARAMS_OK_CONSTRUCTOR, SET_CLIENT_DH_PARAMS_CONSTRUCTOR,
    TELEGRAM_MAIN_RSA_MODULUS_HEX, TELEGRAM_TEST_RSA_MODULUS_HEX,
};
pub use request::{ProtocolRequest, RequestId, RequestSequencer};
pub use schema::{
    audit_mtproto_api_schema, audit_schema, audit_td_api_schema, audit_telegram_api_schema,
    parse_schema_catalog, parse_td_api_schema, DeclarationKind, SchemaAudit, SchemaBaseline,
    SchemaCatalog, SchemaDeclaration, SchemaError, SchemaParseError, SchemaStats,
    MTPROTO_API_BASELINE, MTPROTO_API_EXPECTED_FUNCTIONS, MTPROTO_API_EXPECTED_SHA256,
    MTPROTO_API_EXPECTED_TYPES, TD_API_BASELINE, TD_API_EXPECTED_FUNCTIONS, TD_API_EXPECTED_SHA256,
    TD_API_EXPECTED_TYPES, TD_API_PINNED_COMMIT, TELEGRAM_API_BASELINE,
    TELEGRAM_API_EXPECTED_FUNCTIONS, TELEGRAM_API_EXPECTED_SHA256, TELEGRAM_API_EXPECTED_TYPES,
};
pub use session::{MessageIdError, MessageIdGuard, SequenceError, SessionSequence};
pub use srp::{
    compute_password_srp_proof, compute_password_srp_proof_with_random, PasswordSrpParameters,
    PasswordSrpProof, SrpError, INPUT_CHECK_PASSWORD_SRP_CONSTRUCTOR,
};
pub use tl::{TlError, TlReader, TlWriter, BOOL_FALSE_CONSTRUCTOR, BOOL_TRUE_CONSTRUCTOR};
pub use tls::{
    decode_tls_server_record, encode_tls_client_records, inspect_tls_server_hello,
    prepare_tls_client_hello, prepare_tls_client_hello_with_entropy, tls_camouflage_hmac,
    DecodedTlsRecord, TlsCamouflageError, TlsCamouflageSecret, TlsClientHello,
};
pub use transport::{
    is_good_direct_start_nonce, DecodedFrame, ObfuscatedConnectionStart,
    ObfuscatedTransportSecret, TransportError, TransportFrameCodec, TransportMode,
};
pub use updates::{
    build_updates_get_difference, build_updates_get_state, parse_new_session_created,
    parse_terminal_difference, parse_update_state, DifferenceRequest, NewSessionCreated,
    TerminalDifference, UpdateError, UpdateState,
    UPDATES_DIFFERENCE_EMPTY_CONSTRUCTOR, UPDATES_DIFFERENCE_TOO_LONG_CONSTRUCTOR,
    NEW_SESSION_CREATED_CONSTRUCTOR, UPDATES_GET_DIFFERENCE_CONSTRUCTOR,
    UPDATES_GET_STATE_CONSTRUCTOR, UPDATES_STATE_CONSTRUCTOR,
};

pub use web_proxy::{
    decode_web_proxy_link_secret, encode_web_proxy_link_secret, normalize_web_proxy_base_path,
    parse_web_proxy_secret_text, validate_canonical_web_proxy_host, web_proxy_bridge_capability,
    web_proxy_bridge_path, web_proxy_bridge_url, WebProxyConfigError,
};
