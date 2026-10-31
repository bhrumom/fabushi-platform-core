use fabushi_telegram_protocol::{
    aes_ige_encrypt, build_auth_send_code, build_auth_sign_in, build_auth_sign_up,
    build_client_dh_inner_data, build_init_connection_get_config, build_msg_resend_req, build_msgs_ack, build_msgs_state_req,
    build_p_q_inner_data_dc, build_p_q_inner_data_temp_dc, build_req_dh_params, build_req_pq_multi,
    build_set_client_dh_params,
    decrypt_message, decrypt_server_dh_inner_data, derive_aes_key_iv, derive_tmp_aes_key_iv,
    encrypt_message, factor_res_pq, parse_dh_gen_result, parse_res_pq, parse_schema_catalog,
    parse_server_dh_params, parse_td_api_schema, prepare_client_dh, quick_ack_token,
    rsa_pad_with_random, select_server_key_fingerprint, telegram_server_rsa_key,
    wire_constructor_by_name, wire_constructors_by_id, AuthCommand, AuthError, AuthKey,
    AuthKeyHandshake, AuthKeyHandshakeState, AuthorizationMachine, AuthorizationState,
    CodeDeliveryType, CryptoDirection, CryptoError, DeclarationKind, DhGenAction, DhGenResult,
    EncryptedEnvelope, HandshakeError, MessageIdError, MessageIdGuard, PasswordSrpParameters,
    PlainMessage, PlaintextEnvelope, RequestSequencer, ResPq, RsaPublicKey, SchemaParseError,
    SchemaStats, ServerDhInnerData, ServerDhParams, SessionSequence, TlError, TlReader, TlWriter,
    TransportError, TransportFrameCodec, TransportMode, WireSchema, DH_GEN_FAIL_CONSTRUCTOR,
    DH_GEN_OK_CONSTRUCTOR, DH_GEN_RETRY_CONSTRUCTOR,
    CLIENT_DH_INNER_DATA_CONSTRUCTOR, KNOWN_DH_PRIME_HEX, P_Q_INNER_DATA_DC_CONSTRUCTOR,
    P_Q_INNER_DATA_TEMP_DC_CONSTRUCTOR, REQ_DH_PARAMS_CONSTRUCTOR, REQ_PQ_MULTI_CONSTRUCTOR,
    RES_PQ_CONSTRUCTOR,
    SERVER_DH_INNER_DATA_CONSTRUCTOR,
    SERVER_DH_PARAMS_OK_CONSTRUCTOR, SET_CLIENT_DH_PARAMS_CONSTRUCTOR, WIRE_CONSTRUCTORS,
};
use fabushi_telegram_protocol::{
    build_account_get_password, build_account_send_verify_email_code, build_account_verify_email,
    build_auth_cancel_code, build_auth_check_password, build_auth_check_recovery_password,
    build_auth_request_password_recovery, build_auth_resend_code,
    compute_password_srp_proof_with_random,
    parse_account_email_verified, parse_account_password_prefix, parse_account_sent_email_code,
    parse_auth_password_recovery, parse_auth_sent_code, parse_bool, parse_config_dc_directory_prefix,
    parse_destroy_auth_key_result, try_parse_rpc_error, ApiRequestError, DestroyAuthKeyResult,
    EmailVerifiedResult, InitConnection, NextCodeType,
    SentCode, SentCodeDelivery, SentCodeResult, ACCOUNT_EMAIL_VERIFIED_LOGIN_CONSTRUCTOR,
    ACCOUNT_GET_PASSWORD_CONSTRUCTOR, ACCOUNT_PASSWORD_CONSTRUCTOR,
    AUTH_CHECK_PASSWORD_CONSTRUCTOR, AUTH_SEND_CODE_CONSTRUCTOR, AUTH_SENT_CODE_CONSTRUCTOR,
    AUTH_SIGN_IN_CONSTRUCTOR, AUTH_SIGN_UP_CONSTRUCTOR, CODE_SETTINGS_CONSTRUCTOR,
    CONFIG_CONSTRUCTOR, DC_OPTION_CONSTRUCTOR, DESTROY_AUTH_KEY_FAIL_CONSTRUCTOR,
    DESTROY_AUTH_KEY_NONE_CONSTRUCTOR, DESTROY_AUTH_KEY_OK_CONSTRUCTOR, HELP_GET_CONFIG_CONSTRUCTOR,
    INIT_CONNECTION_CONSTRUCTOR, INPUT_CHECK_PASSWORD_SRP_CONSTRUCTOR,
    INVOKE_WITH_LAYER_CONSTRUCTOR, MSG_RESEND_REQ_CONSTRUCTOR, MSGS_ACK_CONSTRUCTOR, MSGS_STATE_REQ_CONSTRUCTOR, MTPROTO_LAYER,
    PASSWORD_KDF_ALGO_CONSTRUCTOR,
};
use fabushi_telegram_protocol::{
    build_updates_get_difference, build_updates_get_state, parse_terminal_difference,
    parse_update_state, DifferenceRequest, TerminalDifference, UpdateState,
    UPDATES_DIFFERENCE_EMPTY_CONSTRUCTOR, UPDATES_GET_DIFFERENCE_CONSTRUCTOR,
    UPDATES_GET_STATE_CONSTRUCTOR, UPDATES_STATE_CONSTRUCTOR,
};
use fabushi_telegram_protocol::{
    classify_rpc_error, DcDirectory, DcEndpoint, DcError, DcPurpose, DcRoute, MigrationDirective,
    MigrationKind, RpcErrorAction,
};
use num_bigint_dig::BigUint;
use serde_json::{json, Map};
use sha1::{Digest, Sha1};

#[test]
fn authorization_requires_parameters_before_phone_number() {
    let mut machine = AuthorizationMachine::new();
    let error = machine
        .execute(AuthCommand::SubmitPhoneNumber {
            phone_number: "+8613800138000".to_string(),
        })
        .unwrap_err();
    assert!(matches!(error, AuthError::InvalidTransition { .. }));

    machine.execute(AuthCommand::ParametersAccepted).unwrap();
    assert_eq!(machine.state(), &AuthorizationState::WaitPhoneNumber);
    machine
        .execute(AuthCommand::SubmitPhoneNumber {
            phone_number: "+86 138 0013 8000".to_string(),
        })
        .unwrap();
    assert_eq!(machine.state(), &AuthorizationState::WaitPhoneNumber);
}

#[test]
fn update_state_and_difference_requests_preserve_all_cursors() {
    assert_eq!(
        u32::from_le_bytes(build_updates_get_state().try_into().unwrap()),
        UPDATES_GET_STATE_CONSTRUCTOR
    );
    let state = UpdateState {
        pts: 101,
        qts: 202,
        date: 1_720_000_000,
        seq: 303,
        unread_count: 4,
    };
    let request = build_updates_get_difference(DifferenceRequest {
        state,
        pts_limit: Some(1000),
        pts_total_limit: Some(10_000),
        qts_limit: Some(500),
    })
    .unwrap();
    let mut reader = TlReader::new(&request);
    assert_eq!(
        reader.read_u32().unwrap(),
        UPDATES_GET_DIFFERENCE_CONSTRUCTOR
    );
    assert_eq!(reader.read_i32().unwrap(), 7);
    assert_eq!(reader.read_i32().unwrap(), 101);
    assert_eq!(reader.read_i32().unwrap(), 1000);
    assert_eq!(reader.read_i32().unwrap(), 10_000);
    assert_eq!(reader.read_i32().unwrap(), 1_720_000_000);
    assert_eq!(reader.read_i32().unwrap(), 202);
    assert_eq!(reader.read_i32().unwrap(), 500);
    assert!(reader.is_finished());

    let mut encoded_state = TlWriter::new();
    encoded_state.write_u32(UPDATES_STATE_CONSTRUCTOR);
    encoded_state.write_i32(state.pts);
    encoded_state.write_i32(state.qts);
    encoded_state.write_i32(state.date);
    encoded_state.write_i32(state.seq);
    encoded_state.write_i32(state.unread_count);
    assert_eq!(
        parse_update_state(&encoded_state.into_bytes()).unwrap(),
        state
    );

    let mut empty = TlWriter::new();
    empty.write_u32(UPDATES_DIFFERENCE_EMPTY_CONSTRUCTOR);
    empty.write_i32(1_720_000_100);
    empty.write_i32(304);
    assert_eq!(
        parse_terminal_difference(&empty.into_bytes()).unwrap(),
        TerminalDifference::Empty {
            date: 1_720_000_100,
            seq: 304,
        }
    );
}

#[test]
fn api_initialization_and_phone_auth_requests_match_pinned_layer() {
    let request = build_init_connection_get_config(&InitConnection {
        api_id: 12_345,
        device_model: "Fabushi Test",
        system_version: "Rust",
        app_version: "0.1.0",
        system_lang_code: "zh-Hans",
        lang_pack: "",
        lang_code: "zh-hans",
    })
    .unwrap();
    let mut reader = TlReader::new(&request);
    assert_eq!(reader.read_u32().unwrap(), INVOKE_WITH_LAYER_CONSTRUCTOR);
    assert_eq!(reader.read_i32().unwrap(), MTPROTO_LAYER);
    assert_eq!(reader.read_u32().unwrap(), INIT_CONNECTION_CONSTRUCTOR);
    assert_eq!(reader.read_i32().unwrap(), 0);
    assert_eq!(reader.read_i32().unwrap(), 12_345);
    assert_eq!(reader.read_string().unwrap(), "Fabushi Test");
    assert_eq!(reader.read_string().unwrap(), "Rust");
    assert_eq!(reader.read_string().unwrap(), "0.1.0");
    assert_eq!(reader.read_string().unwrap(), "zh-Hans");
    assert_eq!(reader.read_string().unwrap(), "");
    assert_eq!(reader.read_string().unwrap(), "zh-hans");
    assert_eq!(reader.read_u32().unwrap(), HELP_GET_CONFIG_CONSTRUCTOR);
    assert!(reader.is_finished());

    let request = build_auth_send_code("+8613800138000", 12_345, "secret-hash").unwrap();
    let mut reader = TlReader::new(&request);
    assert_eq!(reader.read_u32().unwrap(), AUTH_SEND_CODE_CONSTRUCTOR);
    assert_eq!(reader.read_string().unwrap(), "+8613800138000");
    assert_eq!(reader.read_i32().unwrap(), 12_345);
    assert_eq!(reader.read_string().unwrap(), "secret-hash");
    assert_eq!(reader.read_u32().unwrap(), CODE_SETTINGS_CONSTRUCTOR);
    assert_eq!(reader.read_i32().unwrap(), 0);
    assert!(reader.is_finished());

    let request = build_auth_sign_in("+8613800138000", "code-hash", "12345").unwrap();
    let mut reader = TlReader::new(&request);
    assert_eq!(reader.read_u32().unwrap(), AUTH_SIGN_IN_CONSTRUCTOR);
    assert_eq!(reader.read_i32().unwrap(), 1);
    assert_eq!(reader.read_string().unwrap(), "+8613800138000");
    assert_eq!(reader.read_string().unwrap(), "code-hash");
    assert_eq!(reader.read_string().unwrap(), "12345");
    assert!(reader.is_finished());

    let request = build_auth_sign_up("+8613800138000", "code-hash", "法", "布施").unwrap();
    let mut reader = TlReader::new(&request);
    assert_eq!(reader.read_u32().unwrap(), AUTH_SIGN_UP_CONSTRUCTOR);
    assert_eq!(reader.read_i32().unwrap(), 0);
    assert_eq!(reader.read_string().unwrap(), "+8613800138000");
    assert_eq!(reader.read_string().unwrap(), "code-hash");
    assert_eq!(reader.read_string().unwrap(), "法");
    assert_eq!(reader.read_string().unwrap(), "布施");
    assert!(reader.is_finished());

    assert_eq!(
        build_auth_send_code("13800138000", 12_345, "hash").unwrap_err(),
        ApiRequestError::InvalidPhoneNumber
    );
}

#[test]
fn message_acknowledgement_encodes_server_ids_as_a_tl_vector() {
    let request = build_msgs_ack(&[101, 103]).unwrap();
    let mut reader = TlReader::new(&request);
    assert_eq!(reader.read_u32().unwrap(), MSGS_ACK_CONSTRUCTOR);
    assert_eq!(reader.read_vector_length().unwrap(), 2);
    assert_eq!(reader.read_i64().unwrap(), 101);
    assert_eq!(reader.read_i64().unwrap(), 103);
    assert!(reader.is_finished());
}

#[test]
fn msg_resend_req_request_wire_is_exact_and_vector_bounded() {
    let empty = build_msg_resend_req(&[]).unwrap();
    let mut empty_reader = TlReader::new(&empty);
    assert_eq!(empty_reader.read_u32().unwrap(), MSG_RESEND_REQ_CONSTRUCTOR);
    assert_eq!(empty_reader.read_vector_length().unwrap(), 0);
    assert!(empty_reader.is_finished());

    let shipping_id = 0x0102_0304_0506_0708_i64;
    let one = build_msg_resend_req(&[shipping_id]).unwrap();
    let mut one_expected = Vec::new();
    one_expected.extend_from_slice(&MSG_RESEND_REQ_CONSTRUCTOR.to_le_bytes());
    one_expected.extend_from_slice(&0x1cb5_c415_u32.to_le_bytes());
    one_expected.extend_from_slice(&1_i32.to_le_bytes());
    one_expected.extend_from_slice(&shipping_id.to_le_bytes());
    assert_eq!(one, one_expected);

    let ids = [i64::MIN, -1, 0, 0x1112_1314_1516_1718_i64, i64::MAX];
    let encoded = build_msg_resend_req(&ids).unwrap();
    let mut expected = Vec::new();
    expected.extend_from_slice(&MSG_RESEND_REQ_CONSTRUCTOR.to_le_bytes());
    expected.extend_from_slice(&0x1cb5_c415_u32.to_le_bytes());
    expected.extend_from_slice(&(ids.len() as i32).to_le_bytes());
    for id in ids {
        expected.extend_from_slice(&id.to_le_bytes());
    }
    assert_eq!(encoded, expected);

    let mut reader = TlReader::new(&encoded);
    assert_eq!(reader.read_u32().unwrap(), MSG_RESEND_REQ_CONSTRUCTOR);
    assert_eq!(reader.read_vector_length().unwrap(), ids.len());
    for id in ids {
        assert_eq!(reader.read_i64().unwrap(), id);
    }
    assert!(reader.is_finished());

    let mut overflow = TlWriter::new();
    assert_eq!(
        overflow
            .write_vector_length(i32::MAX as usize + 1)
            .unwrap_err(),
        TlError::LengthOutOfRange
    );
    assert!(overflow.as_slice().is_empty());
}

#[test]
fn msgs_state_req_request_wire_is_exact_and_vector_bounded() {
    let empty = build_msgs_state_req(&[]).unwrap();
    let mut empty_reader = TlReader::new(&empty);
    assert_eq!(empty_reader.read_u32().unwrap(), MSGS_STATE_REQ_CONSTRUCTOR);
    assert_eq!(empty_reader.read_vector_length().unwrap(), 0);
    assert!(empty_reader.is_finished());

    let shipping_id = 0x1112_1314_1516_1718_i64;
    let one = build_msgs_state_req(&[shipping_id]).unwrap();
    let mut one_expected = Vec::new();
    one_expected.extend_from_slice(&MSGS_STATE_REQ_CONSTRUCTOR.to_le_bytes());
    one_expected.extend_from_slice(&0x1cb5_c415_u32.to_le_bytes());
    one_expected.extend_from_slice(&1_i32.to_le_bytes());
    one_expected.extend_from_slice(&shipping_id.to_le_bytes());
    assert_eq!(one, one_expected);

    let ids = [i64::MIN, -1, 0, 0x0102_0304_0506_0708_i64, i64::MAX];
    let encoded = build_msgs_state_req(&ids).unwrap();
    let mut expected = Vec::new();
    expected.extend_from_slice(&MSGS_STATE_REQ_CONSTRUCTOR.to_le_bytes());
    expected.extend_from_slice(&0x1cb5_c415_u32.to_le_bytes());
    expected.extend_from_slice(&(ids.len() as i32).to_le_bytes());
    for id in ids {
        expected.extend_from_slice(&id.to_le_bytes());
    }
    assert_eq!(encoded, expected);

    let mut reader = TlReader::new(&encoded);
    assert_eq!(reader.read_u32().unwrap(), MSGS_STATE_REQ_CONSTRUCTOR);
    assert_eq!(reader.read_vector_length().unwrap(), ids.len());
    for id in ids {
        assert_eq!(reader.read_i64().unwrap(), id);
    }
    assert!(reader.is_finished());

    let mut overflow = TlWriter::new();
    assert_eq!(
        overflow
            .write_vector_length(i32::MAX as usize + 1)
            .unwrap_err(),
        TlError::LengthOutOfRange
    );
    assert!(overflow.as_slice().is_empty());
}

#[test]
fn config_prefix_replaces_bootstrap_directory_with_server_endpoints() {
    let mut writer = TlWriter::new();
    writer.write_u32(CONFIG_CONSTRUCTOR);
    writer.write_i32(0);
    writer.write_i32(1_720_000_000);
    writer.write_i32(1_720_003_600);
    writer.write_bool(false);
    writer.write_i32(2);
    writer.write_vector_length(2).unwrap();
    writer.write_u32(DC_OPTION_CONSTRUCTOR);
    writer.write_i32(16);
    writer.write_i32(2);
    writer.write_string("149.154.167.51").unwrap();
    writer.write_i32(443);
    writer.write_u32(DC_OPTION_CONSTRUCTOR);
    writer.write_i32(1 | 2 | (1 << 10));
    writer.write_i32(2);
    writer.write_string("2001:67c:4e8:f002::a").unwrap();
    writer.write_i32(443);
    writer.write_bytes(&[0xee, 0xaa]).unwrap();
    writer.write_string("trailing-config-field").unwrap();

    let config = parse_config_dc_directory_prefix(&writer.into_bytes()).unwrap();
    assert_eq!(config.date, 1_720_000_000);
    assert_eq!(config.expires, 1_720_003_600);
    assert_eq!(config.directory.this_dc(), 2);
    assert!(!config.directory.test_mode());
    assert_eq!(config.directory.endpoints().len(), 2);
    let ipv6 = config
        .directory
        .endpoints()
        .iter()
        .find(|endpoint| endpoint.ip_address.is_ipv6())
        .unwrap();
    assert!(ipv6.media_only);
    assert_eq!(ipv6.secret.as_deref(), Some(&[0xee, 0xaa][..]));
}

#[test]
fn sent_code_parser_covers_delivery_hash_fallback_and_timeout() {
    let mut writer = TlWriter::new();
    writer.write_u32(AUTH_SENT_CODE_CONSTRUCTOR);
    writer.write_i32(2 | 4);
    writer.write_u32(0x3dbb_5986);
    writer.write_i32(5);
    writer.write_string("phone-code-hash").unwrap();
    writer.write_u32(0x72a3_158c);
    writer.write_i32(60);

    assert_eq!(
        parse_auth_sent_code(&writer.into_bytes()).unwrap(),
        SentCodeResult::Code {
            code: SentCode {
                phone_code_hash: "phone-code-hash".to_string(),
                delivery: SentCodeDelivery::App { length: 5 },
                next_type: Some(NextCodeType::Sms),
                timeout_seconds: Some(60),
            }
        }
    );
}

#[test]
fn rpc_error_parser_preserves_code_and_machine_message() {
    let mut writer = TlWriter::new();
    writer.write_u32(fabushi_telegram_protocol::RPC_ERROR_CONSTRUCTOR);
    writer.write_i32(400);
    writer.write_string("PHONE_CODE_INVALID").unwrap();
    let error = try_parse_rpc_error(&writer.into_bytes()).unwrap().unwrap();
    assert_eq!(error.code, 400);
    assert_eq!(error.message, "PHONE_CODE_INVALID");
}

#[test]
fn rpc_error_parser_rejects_malformed_or_trailing_wire_and_discriminates_other_objects() {
    assert!(try_parse_rpc_error(&0x3477_73c5_u32.to_le_bytes())
        .unwrap()
        .is_none());

    assert!(matches!(
        try_parse_rpc_error(&fabushi_telegram_protocol::RPC_ERROR_CONSTRUCTOR.to_le_bytes()),
        Err(ApiRequestError::Tl(TlError::UnexpectedEof { .. }))
    ));

    let mut invalid_utf8 = TlWriter::new();
    invalid_utf8.write_u32(fabushi_telegram_protocol::RPC_ERROR_CONSTRUCTOR);
    invalid_utf8.write_i32(-500);
    invalid_utf8.write_bytes(&[0xff]).unwrap();
    assert!(matches!(
        try_parse_rpc_error(&invalid_utf8.into_bytes()),
        Err(ApiRequestError::Tl(TlError::InvalidUtf8 { .. }))
    ));

    let mut trailing = TlWriter::new();
    trailing.write_u32(fabushi_telegram_protocol::RPC_ERROR_CONSTRUCTOR);
    trailing.write_i32(-303);
    trailing.write_string("PHONE_MIGRATE_2").unwrap();
    trailing.write_u32(0xfeed_beef);
    assert_eq!(
        try_parse_rpc_error(&trailing.into_bytes()).unwrap_err(),
        ApiRequestError::TrailingBytes(4)
    );

    let mut exact = TlWriter::new();
    exact.write_u32(fabushi_telegram_protocol::RPC_ERROR_CONSTRUCTOR);
    exact.write_i32(i32::MIN);
    exact.write_string("EDGE_CASE").unwrap();
    let exact_bytes = exact.into_bytes();
    let parsed = try_parse_rpc_error(&exact_bytes).unwrap().unwrap();
    assert_eq!(parsed.code, i32::MIN);
    assert_eq!(parsed.message, "EDGE_CASE");

    let mut reader = TlReader::new(&exact_bytes);
    assert_eq!(
        reader.read_u32().unwrap(),
        fabushi_telegram_protocol::RPC_ERROR_CONSTRUCTOR
    );
    assert_eq!(reader.read_i32().unwrap(), i32::MIN);
    assert_eq!(reader.read_string().unwrap(), "EDGE_CASE");
    assert!(reader.is_finished());
}

#[test]
fn account_password_prefix_and_srp_proof_follow_telegram_contract() {
    assert_eq!(
        u32::from_le_bytes(build_account_get_password().try_into().unwrap()),
        ACCOUNT_GET_PASSWORD_CONSTRUCTOR
    );
    let prime = hex::decode(KNOWN_DH_PRIME_HEX).unwrap();
    let prime_number = BigUint::from_bytes_be(&prime);
    let server_b = BigUint::from(3_u8)
        .modpow(&BigUint::from(123_456_u32), &prime_number)
        .to_bytes_be();
    let mut writer = TlWriter::new();
    writer.write_u32(ACCOUNT_PASSWORD_CONSTRUCTOR);
    writer.write_i32(1 | 4 | 8 | 16);
    writer.write_u32(PASSWORD_KDF_ALGO_CONSTRUCTOR);
    writer.write_bytes(b"salt-one").unwrap();
    writer.write_bytes(b"salt-two").unwrap();
    writer.write_i32(3);
    writer.write_bytes(&prime).unwrap();
    writer.write_bytes(&server_b).unwrap();
    writer.write_i64(998_877);
    writer.write_string("hint").unwrap();
    writer.write_string("m***@example.com").unwrap();
    writer.write_u32(0xd45a_b096);

    let state = parse_account_password_prefix(&writer.into_bytes()).unwrap();
    assert!(state.has_recovery);
    assert_eq!(state.hint.as_deref(), Some("hint"));
    let parameters: PasswordSrpParameters = state.srp.unwrap();
    let proof = compute_password_srp_proof_with_random(
        "correct horse battery staple",
        &parameters,
        |output| {
            output.fill(0x11);
            Ok(())
        },
    )
    .unwrap();
    assert_eq!(proof.srp_id, 998_877);
    assert_ne!(proof.a, [0_u8; 256]);
    assert_ne!(proof.m1, [0_u8; 32]);

    let request = build_auth_check_password(&proof).unwrap();
    let mut reader = TlReader::new(&request);
    assert_eq!(reader.read_u32().unwrap(), AUTH_CHECK_PASSWORD_CONSTRUCTOR);
    assert_eq!(
        reader.read_u32().unwrap(),
        INPUT_CHECK_PASSWORD_SRP_CONSTRUCTOR
    );
    assert_eq!(reader.read_i64().unwrap(), 998_877);
    assert_eq!(reader.read_bytes().unwrap(), proof.a);
    assert_eq!(reader.read_bytes().unwrap(), proof.m1);
    assert!(reader.is_finished());
}

#[test]
fn telegram_bootstrap_directory_matches_pinned_tdlib_surface() {
    let production = DcDirectory::telegram_defaults(false, 2).unwrap();
    assert_eq!(production.endpoints().len(), 33);
    let dc_two = production.candidates(2, DcPurpose::Main, false).unwrap();
    assert_eq!(dc_two[0].ip_address.to_string(), "149.154.167.51");
    assert_eq!(dc_two[0].port, 80);
    assert!(dc_two
        .iter()
        .any(|endpoint| endpoint.ip_address.to_string() == "95.161.76.100"));
    assert!(dc_two.iter().any(|endpoint| endpoint.ip_address.is_ipv6()));

    let test = DcDirectory::telegram_defaults(true, 1).unwrap();
    assert_eq!(test.endpoints().len(), 18);
    assert!(matches!(
        test.candidates(4, DcPurpose::Main, false),
        Err(DcError::MissingEndpoint { dc_id: 4, .. })
    ));
}

#[test]
fn auth_key_negotiator_runs_all_wire_phases_to_completion() {
    let nonce = [0x11; 16];
    let server_nonce = [0x22; 16];
    let mut handshake = AuthKeyHandshake::new(nonce);
    handshake.begin(4).unwrap();

    let fingerprint = telegram_server_rsa_key(false)
        .unwrap()
        .fingerprint()
        .unwrap();
    let mut res_pq = TlWriter::new();
    res_pq.write_u32(RES_PQ_CONSTRUCTOR);
    res_pq.write_i128_bytes(&nonce);
    res_pq.write_i128_bytes(&server_nonce);
    res_pq.write_bytes(&[0x04, 0x2a, 0x39]).unwrap();
    res_pq.write_vector_length(1).unwrap();
    res_pq.write_u64(fingerprint);
    handshake
        .receive_res_pq(PlaintextEnvelope {
            message_id: 5,
            body: res_pq.into_bytes(),
        })
        .unwrap();

    let fixed_new_nonce = |output: &mut [u8]| {
        output.fill(0x56);
        Ok(())
    };
    let request = handshake
        .prepare_req_dh_params_with_random(8, 2, false, fixed_new_nonce)
        .unwrap();
    assert_eq!(request.message_id, 8);
    assert_eq!(handshake.state(), &AuthKeyHandshakeState::AwaitingServerDh);

    let new_nonce = [0x56; 32];
    let dh_prime = hex::decode(KNOWN_DH_PRIME_HEX).unwrap();
    let mut g_a = vec![0_u8; 256];
    g_a[0] = 0x80;
    let mut server_inner = TlWriter::new();
    server_inner.write_u32(SERVER_DH_INNER_DATA_CONSTRUCTOR);
    server_inner.write_i128_bytes(&nonce);
    server_inner.write_i128_bytes(&server_nonce);
    server_inner.write_i32(3);
    server_inner.write_bytes(&dh_prime).unwrap();
    server_inner.write_bytes(&g_a).unwrap();
    server_inner.write_i32(1_720_000_123);
    let server_inner = server_inner.into_bytes();
    let mut answer_with_hash = Sha1::digest(&server_inner).to_vec();
    answer_with_hash.extend_from_slice(&server_inner);
    while answer_with_hash.len() % 16 != 0 {
        answer_with_hash.push(0x44);
    }
    let (key, iv) = derive_tmp_aes_key_iv(&new_nonce, &server_nonce);
    let encrypted_answer = aes_ige_encrypt(&answer_with_hash, &key, &iv).unwrap();
    let mut server_dh = TlWriter::new();
    server_dh.write_u32(SERVER_DH_PARAMS_OK_CONSTRUCTOR);
    server_dh.write_i128_bytes(&nonce);
    server_dh.write_i128_bytes(&server_nonce);
    server_dh.write_bytes(&encrypted_answer).unwrap();

    let fixed_private = |output: &mut [u8]| {
        for (index, byte) in output.iter_mut().enumerate() {
            *byte = (index as u8).wrapping_mul(13).wrapping_add(9);
        }
        Ok(())
    };
    let client_dh = handshake
        .receive_server_dh_with_random(
            PlaintextEnvelope {
                message_id: 9,
                body: server_dh.into_bytes(),
            },
            12,
            fixed_private,
        )
        .unwrap();
    assert_eq!(client_dh.message_id, 12);
    assert_eq!(handshake.state(), &AuthKeyHandshakeState::AwaitingDhGen);

    let aux_hash = handshake.pending_auth_key_aux_hash().unwrap();
    let mut hash_material = Vec::from(new_nonce);
    hash_material.push(1);
    hash_material.extend_from_slice(&aux_hash.to_le_bytes());
    let digest = Sha1::digest(&hash_material);
    let ok_hash: [u8; 16] = digest[4..20].try_into().unwrap();
    let mut dh_ok = TlWriter::new();
    dh_ok.write_u32(DH_GEN_OK_CONSTRUCTOR);
    dh_ok.write_i128_bytes(&nonce);
    dh_ok.write_i128_bytes(&server_nonce);
    dh_ok.write_i128_bytes(&ok_hash);
    let action = handshake
        .receive_dh_gen(
            PlaintextEnvelope {
                message_id: 13,
                body: dh_ok.into_bytes(),
            },
            16,
        )
        .unwrap();
    match action {
        DhGenAction::Established(established) => {
            assert_ne!(established.auth_key.id(), 0);
            assert_eq!(established.server_time, 1_720_000_123);
            assert_eq!(established.server_salt, i64::from_le_bytes([0x74; 8]));
        }
        DhGenAction::Retry(_) => panic!("server returned an authenticated success"),
    }
    assert_eq!(handshake.state(), &AuthKeyHandshakeState::Complete);
}

#[test]
fn remote_authorization_updates_are_authoritative() {
    let mut machine = AuthorizationMachine::from_state(AuthorizationState::WaitPhoneNumber);
    machine
        .execute(AuthCommand::ApplyRemoteState {
            state: AuthorizationState::WaitCode {
                phone_number: "+8613800138000".to_string(),
                delivery_type: CodeDeliveryType::TelegramMessage,
                code_length: 5,
                timeout_seconds: Some(60),
            },
        })
        .unwrap();
    machine
        .execute(AuthCommand::SubmitCode {
            code: "12345".to_string(),
        })
        .unwrap();
    machine
        .execute(AuthCommand::ApplyRemoteState {
            state: AuthorizationState::Ready,
        })
        .unwrap();
    assert_eq!(machine.state(), &AuthorizationState::Ready);
}

#[test]
fn invalid_phone_code_and_password_are_rejected_locally() {
    let mut phone = AuthorizationMachine::from_state(AuthorizationState::WaitPhoneNumber);
    assert_eq!(
        phone
            .execute(AuthCommand::SubmitPhoneNumber {
                phone_number: "13800138000".to_string(),
            })
            .unwrap_err(),
        AuthError::InvalidPhoneNumber
    );

    let mut code = AuthorizationMachine::from_state(AuthorizationState::WaitCode {
        phone_number: "+8613800138000".to_string(),
        delivery_type: CodeDeliveryType::Sms,
        code_length: 5,
        timeout_seconds: None,
    });
    assert_eq!(
        code.execute(AuthCommand::SubmitCode {
            code: "12 34".to_string(),
        })
        .unwrap_err(),
        AuthError::InvalidCode
    );

    let mut password = AuthorizationMachine::from_state(AuthorizationState::WaitPassword {
        password_hint: "hint".to_string(),
        has_recovery_email: true,
        recovery_email_pattern: Some("m***@example.com".to_string()),
    });
    assert_eq!(
        password
            .execute(AuthCommand::SubmitPassword {
                password: String::new(),
            })
            .unwrap_err(),
        AuthError::InvalidPassword
    );
}

#[test]
fn request_sequencer_emits_tdlib_compatible_envelopes() {
    let mut sequencer = RequestSequencer::new();
    let mut parameters = Map::new();
    parameters.insert("chat_id".to_string(), json!(42));
    let request = sequencer.create("getChat", parameters).unwrap();
    let td_json = request.to_td_json();
    assert_eq!(td_json["@type"], "getChat");
    assert_eq!(td_json["@extra"], "1");
    assert_eq!(td_json["chat_id"], 42);
}

#[test]
fn schema_parser_counts_multiline_type_and_function_declarations() {
    let sample = r#"
---types---
// comment
thing value:int32
  label:string = Thing;
other = Other; // trailing comment
---functions---
getThing id:int64 = Thing;
setThing id:int64
  value:thing = Ok;
"#;
    assert_eq!(
        parse_td_api_schema(sample),
        SchemaStats {
            types: 2,
            functions: 2,
        }
    );
}

#[test]
fn schema_catalog_extracts_names_results_sections_and_constructor_ids() {
    let sample = r#"
thing#0a0b0c0d value:int32 = Thing;
---functions---
getThing#11223344 id:int64 = Thing;
"#;
    let catalog = parse_schema_catalog(sample).unwrap();
    assert_eq!(
        catalog.stats(),
        SchemaStats {
            types: 1,
            functions: 1
        }
    );
    assert_eq!(catalog.types[0].name, "thing");
    assert_eq!(catalog.types[0].constructor_id, Some(0x0a0b_0c0d));
    assert_eq!(catalog.types[0].result_type, "Thing");
    assert_eq!(catalog.functions[0].kind, DeclarationKind::Function);
    assert_eq!(catalog.functions[0].name, "getThing");
    assert_eq!(catalog.explicit_constructor_count(), 2);

    let duplicate = parse_schema_catalog("first#01020304 = First;\nsecond#01020304 = Second;");
    assert!(matches!(
        duplicate,
        Err(SchemaParseError::DuplicateConstructorId { .. })
    ));
}

#[test]
fn tl_primitives_bool_bytes_and_vectors_round_trip() {
    for payload_length in [0, 1, 2, 3, 253, 254, 300] {
        let payload = vec![0x5a; payload_length];
        let mut writer = TlWriter::new();
        writer.write_i32(-17);
        writer.write_i64(i64::MAX - 4);
        writer.write_f64(3.5);
        writer.write_bool(true);
        writer.write_bool(false);
        writer.write_bytes(&payload).unwrap();
        writer.write_vector_length(3).unwrap();
        for value in [10, 20, 30] {
            writer.write_i32(value);
        }

        let encoded = writer.into_bytes();
        assert_eq!(encoded.len() % 4, 0);
        let mut reader = TlReader::new(&encoded);
        assert_eq!(reader.read_i32().unwrap(), -17);
        assert_eq!(reader.read_i64().unwrap(), i64::MAX - 4);
        assert_eq!(reader.read_f64().unwrap(), 3.5);
        assert!(reader.read_bool().unwrap());
        assert!(!reader.read_bool().unwrap());
        assert_eq!(reader.read_bytes().unwrap(), payload);
        assert_eq!(reader.read_vector_length().unwrap(), 3);
        assert_eq!(reader.read_i32().unwrap(), 10);
        assert_eq!(reader.read_i32().unwrap(), 20);
        assert_eq!(reader.read_i32().unwrap(), 30);
        assert!(reader.is_finished());
    }
}

#[test]
fn tl_decoder_rejects_truncation_invalid_prefix_and_resource_abuse() {
    let mut truncated = TlReader::new(&[254, 1]);
    assert!(matches!(
        truncated.read_bytes(),
        Err(TlError::UnexpectedEof { .. })
    ));

    let mut invalid_prefix = TlReader::new(&[255, 0, 0, 0]);
    assert_eq!(
        invalid_prefix.read_bytes().unwrap_err(),
        TlError::InvalidBytesPrefix
    );

    let mut length_limited = TlReader::with_limits(&[3, 1, 2, 3], 2, 10);
    assert_eq!(
        length_limited.read_bytes().unwrap_err(),
        TlError::BytesLengthLimit {
            length: 3,
            maximum: 2
        }
    );

    let mut vector_writer = TlWriter::new();
    vector_writer.write_vector_length(11).unwrap();
    let vector_bytes = vector_writer.into_bytes();
    let mut vector_limited = TlReader::with_limits(&vector_bytes, 100, 10);
    assert_eq!(
        vector_limited.read_vector_length().unwrap_err(),
        TlError::VectorLengthLimit {
            length: 11,
            maximum: 10
        }
    );
}

#[test]
fn all_mtproto_transport_frames_round_trip_and_report_partial_input() {
    let payload = vec![0x42; 512];
    for mode in [
        TransportMode::Full,
        TransportMode::Abridged,
        TransportMode::Intermediate,
        TransportMode::PaddedIntermediate,
    ] {
        let mut codec = TransportFrameCodec::new(mode);
        assert_eq!(codec.initial_header(), mode.initial_header());
        let frame = if mode == TransportMode::PaddedIntermediate {
            codec.encode_with_padding(&payload, &[1, 2, 3, 4]).unwrap()
        } else {
            codec.encode(&payload).unwrap()
        };
        assert!(codec.decode(&frame[..frame.len() - 1]).unwrap().is_none());
        let decoded = codec.decode(&frame).unwrap().unwrap();
        assert_eq!(decoded.consumed_bytes, frame.len());
        assert_eq!(decoded.quick_ack_token, None);
        if mode == TransportMode::PaddedIntermediate {
            assert_eq!(&decoded.payload[..payload.len()], payload);
            assert_eq!(&decoded.payload[payload.len()..], &[1, 2, 3, 4]);
        } else {
            assert_eq!(decoded.payload, payload);
        }
        assert_eq!(
            decoded.sequence_number,
            (mode == TransportMode::Full).then_some(0)
        );
    }
}

#[test]
fn full_transport_detects_corruption_and_abridged_enforces_alignment() {
    let mut full = TransportFrameCodec::new(TransportMode::Full);
    let mut frame = full.encode(&[9, 8, 7, 6]).unwrap();
    frame[8] ^= 0xff;
    assert!(matches!(
        full.decode(&frame),
        Err(TransportError::CrcMismatch { .. })
    ));

    let mut abridged = TransportFrameCodec::new(TransportMode::Abridged);
    assert_eq!(
        abridged.encode(&[1, 2, 3]).unwrap_err(),
        TransportError::AbridgedPayloadAlignment(3)
    );
    assert_eq!(
        TransportFrameCodec::with_max_frame_bytes(TransportMode::Intermediate, 3)
            .decode(&[4, 0, 0, 0, 1, 2, 3, 4])
            .unwrap_err(),
        TransportError::FrameLengthLimit {
            length: 4,
            maximum: 3
        }
    );
}

#[test]
fn supported_transports_request_and_decode_quick_ack_tokens() {
    let payload = [1, 2, 3, 4];
    let mut abridged = TransportFrameCodec::new(TransportMode::Abridged);
    let abridged_frame = abridged.encode_requesting_quick_ack(&payload, &[]).unwrap();
    assert_eq!(abridged_frame[0], 0x81);
    let ack = abridged
        .decode(&0x8123_4567_u32.to_be_bytes())
        .unwrap()
        .unwrap();
    assert_eq!(ack.quick_ack_token, Some(0x8123_4567));
    assert!(ack.payload.is_empty());

    let mut intermediate = TransportFrameCodec::new(TransportMode::Intermediate);
    let intermediate_frame = intermediate
        .encode_requesting_quick_ack(&payload, &[])
        .unwrap();
    assert_ne!(intermediate_frame[3] & 0x80, 0);
    let ack = intermediate
        .decode(&0x9234_5678_u32.to_le_bytes())
        .unwrap()
        .unwrap();
    assert_eq!(ack.quick_ack_token, Some(0x9234_5678));

    let padded = TransportFrameCodec::new(TransportMode::PaddedIntermediate);
    let mut padded_ack = Vec::new();
    padded_ack.extend_from_slice(&8_u32.to_le_bytes());
    padded_ack.extend_from_slice(&[0xff; 4]);
    padded_ack.extend_from_slice(&0xa123_4567_u32.to_le_bytes());
    let ack = padded.decode(&padded_ack).unwrap().unwrap();
    assert_eq!(ack.quick_ack_token, Some(0xa123_4567));

    let mut full = TransportFrameCodec::new(TransportMode::Full);
    assert_eq!(
        full.encode_requesting_quick_ack(&payload, &[]).unwrap_err(),
        TransportError::QuickAckUnsupported
    );
}

#[test]
fn generated_wire_constructor_catalog_covers_both_pinned_schemas() {
    assert_eq!(WIRE_CONSTRUCTORS.len(), 2_458);
    let bool_true = wire_constructor_by_name(WireSchema::TelegramApi, "boolTrue").unwrap();
    assert_eq!(bool_true.id, 0x9972_75b5);
    assert_eq!(bool_true.result_type, "Bool");
    let request = wire_constructor_by_name(WireSchema::MtprotoApi, "req_pq_multi").unwrap();
    assert_eq!(request.kind, DeclarationKind::Function);
    let aliases: Vec<_> = wire_constructors_by_id(0xdd28_9f8e).collect();
    assert_eq!(aliases.len(), 2);
    assert!(aliases
        .iter()
        .any(|constructor| constructor.name == "invokeWithBusinessConnectionPrefix"));
    assert!(aliases
        .iter()
        .any(|constructor| constructor.name == "invokeWithBusinessConnection"));
}

#[test]
fn mtproto2_encryption_round_trips_and_rejects_tampering() {
    let key_bytes: Vec<u8> = (0_u16..256).map(|value| value as u8).collect();
    let auth_key = AuthKey::from_slice(&key_bytes).unwrap();
    let message = PlainMessage {
        server_salt: 0x0102_0304_0506_0708,
        session_id: 0x1112_1314_1516_1718,
        message_id: 0x2122_2324_2526_2728,
        sequence_number: 1,
        body: vec![0x44, 0x33, 0x22, 0x11],
        padding: (0_u8..12).collect(),
    };
    let envelope = encrypt_message(&auth_key, CryptoDirection::ClientToServer, &message).unwrap();
    assert_eq!(auth_key.id(), 0xc8df_57a4_6e58_d132);
    assert_eq!(
        hex::encode(envelope.message_key),
        "1eccaffbe28ae4b4d9dc99b7404e381c"
    );
    let (aes_key, aes_iv) = derive_aes_key_iv(
        &auth_key,
        &envelope.message_key,
        CryptoDirection::ClientToServer,
    );
    assert_eq!(
        hex::encode(aes_key),
        "08e5f349f685c2b0559cde71628dad596ba35d275135badcaaf3e4eb6eaafe9c"
    );
    assert_eq!(
        hex::encode(aes_iv),
        "10fcca8c572d38a687004fc4c88da9d4adb4c4cc0691e3e664312c726d49e74f"
    );
    assert_eq!(
        quick_ack_token(&auth_key, CryptoDirection::ClientToServer, &message).unwrap(),
        0xa040_baef
    );
    assert_eq!(envelope.encrypted_data.len() % 16, 0);
    assert_eq!(
        EncryptedEnvelope::from_bytes(&envelope.to_bytes()).unwrap(),
        envelope
    );
    assert_eq!(
        decrypt_message(
            &auth_key,
            CryptoDirection::ClientToServer,
            &envelope,
            message.session_id,
        )
        .unwrap(),
        message
    );

    let mut tampered = envelope.clone();
    tampered.encrypted_data[0] ^= 0x80;
    assert_eq!(
        decrypt_message(
            &auth_key,
            CryptoDirection::ClientToServer,
            &tampered,
            message.session_id,
        )
        .unwrap_err(),
        CryptoError::AuthenticationFailed
    );
}

#[test]
fn mtproto2_decryption_checks_session_and_direction() {
    let auth_key = AuthKey::from_slice(&[7_u8; 256]).unwrap();
    let server_message = PlainMessage {
        server_salt: 3,
        session_id: 4,
        message_id: 5,
        sequence_number: 1,
        body: vec![1, 2, 3, 4],
        padding: vec![9; 12],
    };
    let envelope =
        encrypt_message(&auth_key, CryptoDirection::ServerToClient, &server_message).unwrap();
    assert_eq!(
        decrypt_message(&auth_key, CryptoDirection::ServerToClient, &envelope, 99,).unwrap_err(),
        CryptoError::SessionMismatch
    );

    let invalid_client_message = PlainMessage {
        message_id: 5,
        ..server_message
    };
    let invalid_envelope = encrypt_message(
        &auth_key,
        CryptoDirection::ClientToServer,
        &invalid_client_message,
    )
    .unwrap();
    assert_eq!(
        decrypt_message(
            &auth_key,
            CryptoDirection::ClientToServer,
            &invalid_envelope,
            invalid_client_message.session_id,
        )
        .unwrap_err(),
        CryptoError::InvalidMessageId
    );
}

#[test]
fn message_ids_are_monotonic_time_bound_and_replay_protected() {
    let mut guard = MessageIdGuard::new(2);
    let first = guard
        .generate_client_message_id(1_700_000_000, 123_456_789)
        .unwrap();
    let second = guard
        .generate_client_message_id(1_700_000_000, 123_456_789)
        .unwrap();
    assert_eq!(first.rem_euclid(4), 0);
    assert_eq!(second, first + 4);
    assert_ne!(first & 0xffff_ffff, 0);

    let now = 1_700_000_000_i64;
    let server_one = (now << 32) | 1;
    let server_two = (now << 32) | 3;
    let server_three = (now << 32) | 5;
    guard.validate_server_message_id(server_one, now).unwrap();
    assert_eq!(
        guard
            .validate_server_message_id(server_one, now)
            .unwrap_err(),
        MessageIdError::Replay
    );
    guard.validate_server_message_id(server_two, now).unwrap();
    guard.validate_server_message_id(server_three, now).unwrap();
    assert_eq!(guard.recent_server_message_ids().len(), 2);
    assert_eq!(
        guard
            .validate_server_message_id((now.saturating_sub(301) << 32) | 1, now)
            .unwrap_err(),
        MessageIdError::OutsideTimeWindow
    );
}

#[test]
fn sequence_numbers_follow_content_related_rules() {
    let mut sequence = SessionSequence::new();
    assert_eq!(sequence.next(false).unwrap(), 0);
    assert_eq!(sequence.next(true).unwrap(), 1);
    assert_eq!(sequence.next(false).unwrap(), 2);
    assert_eq!(sequence.next(true).unwrap(), 3);
    assert_eq!(sequence.content_related_count(), 2);
}

#[test]
fn req_pq_multi_builder_serializes_fixed_u_nonce_exactly() {
    let nonce = [
        0x00, 0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77,
        0x88, 0x99, 0xaa, 0xbb, 0xcc, 0xdd, 0xee, 0xff,
    ];
    let encoded = build_req_pq_multi(&nonce);
    assert_eq!(encoded.len(), 20);
    assert_eq!(&encoded[0..4], &REQ_PQ_MULTI_CONSTRUCTOR.to_le_bytes());
    assert_eq!(&encoded[4..20], &nonce);
}

#[test]
fn req_pq_multi_builder_preserves_raw_int128_bytes_without_tl_bytes_framing() {
    for nonce in [[0_u8; 16], [0xff_u8; 16]] {
        let encoded = build_req_pq_multi(&nonce);
        let mut expected = Vec::with_capacity(20);
        expected.extend_from_slice(&REQ_PQ_MULTI_CONSTRUCTOR.to_le_bytes());
        expected.extend_from_slice(&nonce);
        assert_eq!(encoded, expected);
    }
}

#[test]
fn plaintext_envelope_and_req_pq_multi_begin_auth_key_exchange() {
    let nonce = [0x11; 16];
    let mut handshake = AuthKeyHandshake::new(nonce);
    let request = handshake.begin(0x0102_0304_0506_0708).unwrap();
    assert_eq!(request.body.len(), 20);
    assert_eq!(
        u32::from_le_bytes(request.body[0..4].try_into().unwrap()),
        REQ_PQ_MULTI_CONSTRUCTOR
    );
    assert_eq!(&request.body[4..], &nonce);
    assert_eq!(request.body, build_req_pq_multi(&nonce));
    assert_eq!(handshake.state(), &AuthKeyHandshakeState::AwaitingResPq);

    let encoded = request.encode().unwrap();
    assert_eq!(&encoded[0..8], &0_i64.to_le_bytes());
    assert_eq!(PlaintextEnvelope::decode(&encoded).unwrap(), request);

    let mut non_zero_auth_key = encoded.clone();
    non_zero_auth_key[0] = 1;
    assert_eq!(
        PlaintextEnvelope::decode(&non_zero_auth_key).unwrap_err(),
        HandshakeError::UnexpectedAuthKeyId(1)
    );
    assert_eq!(
        PlaintextEnvelope::decode(&encoded[..encoded.len() - 1]).unwrap_err(),
        HandshakeError::EnvelopeLengthMismatch {
            declared: 20,
            actual: 19
        }
    );
}

#[test]
fn res_pq_validates_nonce_pq_and_server_key_fingerprints() {
    let nonce = [0x31; 16];
    let server_nonce = [0x42; 16];
    let mut response = TlWriter::new();
    response.write_u32(RES_PQ_CONSTRUCTOR);
    response.write_i128_bytes(&nonce);
    response.write_i128_bytes(&server_nonce);
    response.write_bytes(&[0x17, 0xed]).unwrap();
    response.write_vector_length(2).unwrap();
    response.write_u64(0x0102_0304_0506_0708);
    response.write_u64(0x8877_6655_4433_2211);

    let mut handshake = AuthKeyHandshake::new(nonce);
    handshake.begin(4).unwrap();
    let parsed = handshake
        .receive_res_pq(PlaintextEnvelope {
            message_id: 5,
            body: response.into_bytes(),
        })
        .unwrap();
    assert_eq!(parsed.server_nonce, server_nonce);
    assert_eq!(parsed.pq, vec![0x17, 0xed]);
    assert_eq!(
        parsed.server_public_key_fingerprints,
        vec![0x0102_0304_0506_0708, 0x8877_6655_4433_2211]
    );

    let mut wrong_nonce_body = TlWriter::new();
    wrong_nonce_body.write_u32(RES_PQ_CONSTRUCTOR);
    wrong_nonce_body.write_i128_bytes(&[0x99; 16]);
    wrong_nonce_body.write_i128_bytes(&server_nonce);
    wrong_nonce_body.write_bytes(&[0x17, 0xed]).unwrap();
    wrong_nonce_body.write_vector_length(1).unwrap();
    wrong_nonce_body.write_u64(7);
    let mut mismatched = AuthKeyHandshake::new(nonce);
    mismatched.begin(8).unwrap();
    assert_eq!(
        mismatched
            .receive_res_pq(PlaintextEnvelope {
                message_id: 9,
                body: wrong_nonce_body.into_bytes(),
            })
            .unwrap_err(),
        HandshakeError::NonceMismatch
    );
}

#[test]
fn res_pq_parser_is_exact_for_fixed_u_body_and_rejects_wire_mutations() {
    let nonce = [0x31; 16];
    let server_nonce = [0x42; 16];
    let mut exact = TlWriter::new();
    exact.write_u32(RES_PQ_CONSTRUCTOR);
    exact.write_i128_bytes(&nonce);
    exact.write_i128_bytes(&server_nonce);
    exact.write_bytes(&[0x04, 0x2a, 0x39]).unwrap();
    exact.write_vector_length(2).unwrap();
    exact.write_u64(0x0102_0304_0506_0708);
    exact.write_u64(0x8877_6655_4433_2211);
    let exact = exact.into_bytes();

    assert_eq!(
        parse_res_pq(&exact).unwrap(),
        ResPq {
            nonce,
            server_nonce,
            pq: vec![0x04, 0x2a, 0x39],
            server_public_key_fingerprints: vec![
                0x0102_0304_0506_0708,
                0x8877_6655_4433_2211,
            ],
        }
    );

    for cut in 0..exact.len() {
        assert!(
            parse_res_pq(&exact[..cut]).is_err(),
            "resPQ truncation cut {cut} unexpectedly accepted"
        );
    }

    assert_eq!(
        parse_res_pq(&0x1122_3344_u32.to_le_bytes()).unwrap_err(),
        HandshakeError::UnexpectedConstructor(0x1122_3344)
    );

    let vector_offset = 4 + 16 + 16 + 4;
    let mut wrong_vector = exact.clone();
    wrong_vector[vector_offset..vector_offset + 4]
        .copy_from_slice(&0x1122_3344_u32.to_le_bytes());
    assert_eq!(
        parse_res_pq(&wrong_vector).unwrap_err(),
        HandshakeError::Tl(TlError::UnexpectedConstructor {
            expected: 0x1cb5_c415,
            actual: 0x1122_3344,
        })
    );

    let mut negative_vector = exact.clone();
    negative_vector[vector_offset + 4..vector_offset + 8]
        .copy_from_slice(&(-1_i32).to_le_bytes());
    assert_eq!(
        parse_res_pq(&negative_vector).unwrap_err(),
        HandshakeError::Tl(TlError::NegativeVectorLength(-1))
    );

    let mut oversized_vector = exact.clone();
    oversized_vector[vector_offset + 4..vector_offset + 8]
        .copy_from_slice(&65_i32.to_le_bytes());
    assert_eq!(
        parse_res_pq(&oversized_vector).unwrap_err(),
        HandshakeError::Tl(TlError::VectorLengthLimit {
            length: 65,
            maximum: 64,
        })
    );

    let mut short_vector = exact.clone();
    short_vector[vector_offset + 4..vector_offset + 8]
        .copy_from_slice(&3_i32.to_le_bytes());
    assert!(matches!(
        parse_res_pq(&short_vector).unwrap_err(),
        HandshakeError::Tl(TlError::UnexpectedEof { .. })
    ));

    let mut trailing = exact.clone();
    trailing.push(0x99);
    assert_eq!(
        parse_res_pq(&trailing).unwrap_err(),
        HandshakeError::TrailingBodyBytes(1)
    );
}

#[test]
fn res_pq_receiver_rejects_empty_fields_nonce_mismatch_and_invalid_state_without_mutation() {
    let nonce = [0x31; 16];
    let server_nonce = [0x42; 16];

    let encode = |body_nonce: [u8; 16], pq: &[u8], fingerprints: &[u64]| {
        let mut writer = TlWriter::new();
        writer.write_u32(RES_PQ_CONSTRUCTOR);
        writer.write_i128_bytes(&body_nonce);
        writer.write_i128_bytes(&server_nonce);
        writer.write_bytes(pq).unwrap();
        writer.write_vector_length(fingerprints.len()).unwrap();
        for fingerprint in fingerprints {
            writer.write_u64(*fingerprint);
        }
        writer.into_bytes()
    };

    let mut empty_pq = AuthKeyHandshake::new(nonce);
    empty_pq.begin(4).unwrap();
    assert_eq!(
        empty_pq
            .receive_res_pq(PlaintextEnvelope {
                message_id: 5,
                body: encode(nonce, &[], &[7]),
            })
            .unwrap_err(),
        HandshakeError::EmptyPq
    );
    assert_eq!(empty_pq.state(), &AuthKeyHandshakeState::AwaitingResPq);

    let mut empty_fingerprints = AuthKeyHandshake::new(nonce);
    empty_fingerprints.begin(6).unwrap();
    assert_eq!(
        empty_fingerprints
            .receive_res_pq(PlaintextEnvelope {
                message_id: 7,
                body: encode(nonce, &[0x17, 0xed], &[]),
            })
            .unwrap_err(),
        HandshakeError::MissingServerKeyFingerprint
    );
    assert_eq!(
        empty_fingerprints.state(),
        &AuthKeyHandshakeState::AwaitingResPq
    );

    let mut wrong_nonce = AuthKeyHandshake::new(nonce);
    wrong_nonce.begin(8).unwrap();
    assert_eq!(
        wrong_nonce
            .receive_res_pq(PlaintextEnvelope {
                message_id: 9,
                body: encode([0x99; 16], &[0x17, 0xed], &[7]),
            })
            .unwrap_err(),
        HandshakeError::NonceMismatch
    );
    assert_eq!(wrong_nonce.state(), &AuthKeyHandshakeState::AwaitingResPq);

    let mut wrong_state = AuthKeyHandshake::new(nonce);
    assert_eq!(
        wrong_state
            .receive_res_pq(PlaintextEnvelope {
                message_id: 10,
                body: encode(nonce, &[0x17, 0xed], &[7]),
            })
            .unwrap_err(),
        HandshakeError::InvalidState("not awaiting resPQ")
    );
    assert_eq!(wrong_state.state(), &AuthKeyHandshakeState::Ready);
}

#[test]
fn res_pq_factors_semiprime_and_builds_next_dh_request_contracts() {
    let response = ResPq {
        nonce: [0x10; 16],
        server_nonce: [0x20; 16],
        // 272953 = 499 * 547, encoded as unsigned big-endian bytes.
        pq: vec![0x04, 0x2a, 0x39],
        server_public_key_fingerprints: vec![0x1111, 0x2222],
    };
    let factors = factor_res_pq(&response).unwrap();
    assert_eq!((factors.p, factors.q), (499, 547));
    assert_eq!(factors.p_bytes(), vec![0x01, 0xf3]);
    assert_eq!(factors.q_bytes(), vec![0x02, 0x23]);
    assert_eq!(
        select_server_key_fingerprint(&response, &[0x9999, 0x2222]).unwrap(),
        0x2222
    );
    assert_eq!(
        select_server_key_fingerprint(&response, &[0x9999]).unwrap_err(),
        HandshakeError::NoTrustedServerKey
    );

    let inner = build_p_q_inner_data_dc(&response, &factors, &[0x30; 32], 2).unwrap();
    let mut inner_reader = TlReader::new(&inner);
    assert_eq!(
        inner_reader.read_u32().unwrap(),
        P_Q_INNER_DATA_DC_CONSTRUCTOR
    );
    assert_eq!(inner_reader.read_bytes().unwrap(), response.pq);
    assert_eq!(inner_reader.read_bytes().unwrap(), factors.p_bytes());
    assert_eq!(inner_reader.read_bytes().unwrap(), factors.q_bytes());
    assert_eq!(inner_reader.read_i128_bytes().unwrap(), response.nonce);
    assert_eq!(
        inner_reader.read_i128_bytes().unwrap(),
        response.server_nonce
    );
    assert_eq!(inner_reader.read_i256_bytes().unwrap(), [0x30; 32]);
    assert_eq!(inner_reader.read_i32().unwrap(), 2);
    assert!(inner_reader.is_finished());

    let temporary =
        build_p_q_inner_data_temp_dc(&response, &factors, &[0x30; 32], 2, 86_400).unwrap();
    let mut temporary_reader = TlReader::new(&temporary);
    assert_eq!(
        temporary_reader.read_u32().unwrap(),
        P_Q_INNER_DATA_TEMP_DC_CONSTRUCTOR
    );
    assert_eq!(temporary_reader.read_bytes().unwrap(), response.pq);
    assert_eq!(temporary_reader.read_bytes().unwrap(), factors.p_bytes());
    assert_eq!(temporary_reader.read_bytes().unwrap(), factors.q_bytes());
    assert_eq!(temporary_reader.read_i128_bytes().unwrap(), response.nonce);
    assert_eq!(
        temporary_reader.read_i128_bytes().unwrap(),
        response.server_nonce
    );
    assert_eq!(temporary_reader.read_i256_bytes().unwrap(), [0x30; 32]);
    assert_eq!(temporary_reader.read_i32().unwrap(), 2);
    assert_eq!(temporary_reader.read_i32().unwrap(), 86_400);
    assert!(temporary_reader.is_finished());
    assert_eq!(
        build_p_q_inner_data_temp_dc(&response, &factors, &[0x30; 32], 2, 0).unwrap_err(),
        HandshakeError::InvalidTemporaryKeyLifetime(0)
    );

    let encrypted = vec![0x55; 256];
    let request = build_req_dh_params(&response, &factors, 0x2222, &encrypted).unwrap();
    let mut request_reader = TlReader::new(&request);
    assert_eq!(
        request_reader.read_u32().unwrap(),
        REQ_DH_PARAMS_CONSTRUCTOR
    );
    assert_eq!(request_reader.read_i128_bytes().unwrap(), response.nonce);
    assert_eq!(
        request_reader.read_i128_bytes().unwrap(),
        response.server_nonce
    );
    assert_eq!(request_reader.read_bytes().unwrap(), factors.p_bytes());
    assert_eq!(request_reader.read_bytes().unwrap(), factors.q_bytes());
    assert_eq!(request_reader.read_u64().unwrap(), 0x2222);
    assert_eq!(request_reader.read_bytes().unwrap(), encrypted);
    assert!(request_reader.is_finished());
}

#[test]
fn temporary_req_dh_preparation_reuses_handshake_state_and_rejects_invalid_lifetime() {
    let nonce = [0x11; 16];
    let server_nonce = [0x22; 16];
    let fingerprint = telegram_server_rsa_key(false)
        .unwrap()
        .fingerprint()
        .unwrap();
    let mut response = TlWriter::new();
    response.write_u32(RES_PQ_CONSTRUCTOR);
    response.write_i128_bytes(&nonce);
    response.write_i128_bytes(&server_nonce);
    response.write_bytes(&[0x04, 0x2a, 0x39]).unwrap();
    response.write_vector_length(1).unwrap();
    response.write_u64(fingerprint);

    let mut handshake = AuthKeyHandshake::new(nonce);
    handshake.begin(4).unwrap();
    handshake
        .receive_res_pq(PlaintextEnvelope {
            message_id: 5,
            body: response.into_bytes(),
        })
        .unwrap();

    let error = handshake
        .prepare_temporary_req_dh_params_with_random(8, 2, false, 0, |output| {
            output.fill(0x56);
            Ok(())
        })
        .unwrap_err();
    assert_eq!(error, HandshakeError::InvalidTemporaryKeyLifetime(0));
    assert!(matches!(
        handshake.state(),
        AuthKeyHandshakeState::ResPqReceived(_)
    ));

    let request = handshake
        .prepare_temporary_req_dh_params_with_random(9, 2, false, 86_400, |output| {
            output.fill(0x56);
            Ok(())
        })
        .unwrap();
    assert_eq!(request.message_id, 9);
    assert_eq!(
        u32::from_le_bytes(request.body[0..4].try_into().unwrap()),
        REQ_DH_PARAMS_CONSTRUCTOR
    );
    assert_eq!(handshake.state(), &AuthKeyHandshakeState::AwaitingServerDh);
}

#[test]
fn rsa_pad_is_deterministic_with_fixed_entropy_and_enforces_payload_limit() {
    assert_eq!(
        telegram_server_rsa_key(false)
            .unwrap()
            .fingerprint()
            .unwrap(),
        0xd09d_1d85_de64_fd85
    );
    assert_eq!(
        telegram_server_rsa_key(true)
            .unwrap()
            .fingerprint()
            .unwrap(),
        0xb258_98df_208d_2603
    );
    let key = RsaPublicKey::from_components(&[0xff; 256], &[0x01, 0x00, 0x01]).unwrap();
    assert_ne!(key.fingerprint().unwrap(), 0);
    let inner_data = vec![0x41; 144];
    let fill = |output: &mut [u8]| {
        for (index, byte) in output.iter_mut().enumerate() {
            *byte = (index as u8).wrapping_mul(17).wrapping_add(3);
        }
        Ok(())
    };
    let first = rsa_pad_with_random(&inner_data, &key, fill).unwrap();
    let second = rsa_pad_with_random(&inner_data, &key, fill).unwrap();
    assert_eq!(first.len(), 256);
    assert_eq!(first, second);
    assert!(first.iter().any(|byte| *byte != 0));
    assert_eq!(
        rsa_pad_with_random(&[0; 145], &key, fill).unwrap_err(),
        HandshakeError::RsaPadDataTooLong(145)
    );
}

#[test]
fn server_dh_params_union_parses_both_fixed_u_variants_exactly() {
    let nonce = [0x12; 16];
    let server_nonce = [0x34; 16];
    let encrypted_answer = [0x10, 0x20, 0x30, 0x40, 0x50];

    let mut ok = TlWriter::new();
    ok.write_u32(SERVER_DH_PARAMS_OK_CONSTRUCTOR);
    ok.write_i128_bytes(&nonce);
    ok.write_i128_bytes(&server_nonce);
    ok.write_bytes(&encrypted_answer).unwrap();
    assert_eq!(
        parse_server_dh_params(ok.as_slice(), &nonce, &server_nonce).unwrap(),
        ServerDhParams::Ok {
            encrypted_answer: encrypted_answer.to_vec(),
        }
    );

    let new_nonce_hash = [0xa5; 16];
    let mut fail = TlWriter::new();
    fail.write_u32(SERVER_DH_PARAMS_FAIL_CONSTRUCTOR);
    fail.write_i128_bytes(&nonce);
    fail.write_i128_bytes(&server_nonce);
    fail.write_i128_bytes(&new_nonce_hash);
    assert_eq!(
        parse_server_dh_params(fail.as_slice(), &nonce, &server_nonce).unwrap(),
        ServerDhParams::Fail { new_nonce_hash }
    );
}

#[test]
fn server_dh_params_union_rejects_unknown_truncation_nonce_mismatch_and_trailing_bytes() {
    let nonce = [0x12; 16];
    let server_nonce = [0x34; 16];
    let unknown = 0x0102_0304_u32;
    assert_eq!(
        parse_server_dh_params(&unknown.to_le_bytes(), &nonce, &server_nonce).unwrap_err(),
        HandshakeError::UnexpectedServerDhConstructor(unknown)
    );

    let mut ok = TlWriter::new();
    ok.write_u32(SERVER_DH_PARAMS_OK_CONSTRUCTOR);
    ok.write_i128_bytes(&nonce);
    ok.write_i128_bytes(&server_nonce);
    ok.write_bytes(&[0x10, 0x20, 0x30, 0x40, 0x50]).unwrap();
    let ok = ok.into_bytes();
    for cut in 0..ok.len() {
        assert!(parse_server_dh_params(&ok[..cut], &nonce, &server_nonce).is_err());
    }

    let mut fail = TlWriter::new();
    fail.write_u32(SERVER_DH_PARAMS_FAIL_CONSTRUCTOR);
    fail.write_i128_bytes(&nonce);
    fail.write_i128_bytes(&server_nonce);
    fail.write_i128_bytes(&[0xa5; 16]);
    let fail = fail.into_bytes();
    for cut in 0..fail.len() {
        assert!(parse_server_dh_params(&fail[..cut], &nonce, &server_nonce).is_err());
    }

    let mut wrong_nonce = TlWriter::new();
    wrong_nonce.write_u32(SERVER_DH_PARAMS_FAIL_CONSTRUCTOR);
    wrong_nonce.write_i128_bytes(&[0x99; 16]);
    wrong_nonce.write_i128_bytes(&server_nonce);
    wrong_nonce.write_i128_bytes(&[0xa5; 16]);
    assert_eq!(
        parse_server_dh_params(wrong_nonce.as_slice(), &nonce, &server_nonce).unwrap_err(),
        HandshakeError::NonceMismatch
    );

    let mut wrong_server_nonce = TlWriter::new();
    wrong_server_nonce.write_u32(SERVER_DH_PARAMS_FAIL_CONSTRUCTOR);
    wrong_server_nonce.write_i128_bytes(&nonce);
    wrong_server_nonce.write_i128_bytes(&[0x88; 16]);
    wrong_server_nonce.write_i128_bytes(&[0xa5; 16]);
    assert_eq!(
        parse_server_dh_params(wrong_server_nonce.as_slice(), &nonce, &server_nonce).unwrap_err(),
        HandshakeError::ServerNonceMismatch
    );

    let mut trailing_ok = ok;
    trailing_ok.push(0);
    assert_eq!(
        parse_server_dh_params(&trailing_ok, &nonce, &server_nonce).unwrap_err(),
        HandshakeError::TrailingBodyBytes(1)
    );

    let mut trailing_fail = fail;
    trailing_fail.push(0);
    assert_eq!(
        parse_server_dh_params(&trailing_fail, &nonce, &server_nonce).unwrap_err(),
        HandshakeError::TrailingBodyBytes(1)
    );
}

#[test]
fn server_dh_answer_is_nonce_bound_decrypted_and_hash_authenticated() {
    let nonce = [0x12; 16];
    let server_nonce = [0x34; 16];
    let new_nonce = [0x56; 32];
    let dh_prime = hex::decode(KNOWN_DH_PRIME_HEX).unwrap();
    let mut g_a = vec![0_u8; 256];
    g_a[0] = 0x80;
    let mut answer = TlWriter::new();
    answer.write_u32(SERVER_DH_INNER_DATA_CONSTRUCTOR);
    answer.write_i128_bytes(&nonce);
    answer.write_i128_bytes(&server_nonce);
    answer.write_i32(3);
    answer.write_bytes(&dh_prime).unwrap();
    answer.write_bytes(&g_a).unwrap();
    answer.write_i32(1_720_000_000);
    let answer = answer.into_bytes();

    let mut plaintext = Sha1::digest(&answer).to_vec();
    plaintext.extend_from_slice(&answer);
    while plaintext.len() % 16 != 0 {
        plaintext.push(0xa5);
    }
    let (key, iv) = derive_tmp_aes_key_iv(&new_nonce, &server_nonce);
    let encrypted = aes_ige_encrypt(&plaintext, &key, &iv).unwrap();

    let mut outer = TlWriter::new();
    outer.write_u32(SERVER_DH_PARAMS_OK_CONSTRUCTOR);
    outer.write_i128_bytes(&nonce);
    outer.write_i128_bytes(&server_nonce);
    outer.write_bytes(&encrypted).unwrap();
    let parsed = parse_server_dh_params(&outer.into_bytes(), &nonce, &server_nonce).unwrap();
    assert_eq!(
        parsed,
        ServerDhParams::Ok {
            encrypted_answer: encrypted.clone()
        }
    );

    let inner =
        decrypt_server_dh_inner_data(&encrypted, &new_nonce, &nonce, &server_nonce).unwrap();
    assert_eq!(inner.generator, 3);
    assert_eq!(inner.dh_prime, dh_prime);
    assert_eq!(inner.g_a, g_a);
    assert_eq!(inner.server_time, 1_720_000_000);

    plaintext[0] ^= 1;
    let bad_encrypted = aes_ige_encrypt(&plaintext, &key, &iv).unwrap();
    assert_eq!(
        decrypt_server_dh_inner_data(&bad_encrypted, &new_nonce, &nonce, &server_nonce)
            .unwrap_err(),
        HandshakeError::ServerDhHashMismatch
    );
}


#[test]
fn server_dh_inner_data_rejects_malformed_wire_and_cryptographic_boundaries() {
    let nonce = [0x12; 16];
    let server_nonce = [0x34; 16];
    let new_nonce = [0x56; 32];
    let dh_prime = hex::decode(KNOWN_DH_PRIME_HEX).unwrap();
    let mut g_a = vec![0_u8; 256];
    g_a[0] = 0x80;

    let encode_inner = |
        constructor: u32,
        body_nonce: [u8; 16],
        body_server_nonce: [u8; 16],
        generator: i32,
        prime: &[u8],
        public_value: &[u8],
        server_time: i32,
    | {
        let mut answer = TlWriter::new();
        answer.write_u32(constructor);
        answer.write_i128_bytes(&body_nonce);
        answer.write_i128_bytes(&body_server_nonce);
        answer.write_i32(generator);
        answer.write_bytes(prime).unwrap();
        answer.write_bytes(public_value).unwrap();
        answer.write_i32(server_time);
        answer.into_bytes()
    };
    let encrypt_inner = |answer: &[u8], residual_padding: usize, corrupt_hash: bool| {
        let mut digest = Sha1::digest(answer).to_vec();
        if corrupt_hash {
            digest[0] ^= 1;
        }
        let mut plaintext = digest;
        plaintext.extend_from_slice(answer);
        plaintext.extend(std::iter::repeat_n(0xa5, residual_padding));
        while plaintext.len() % 16 != 0 {
            plaintext.push(0x5a);
        }
        let (key, iv) = derive_tmp_aes_key_iv(&new_nonce, &server_nonce);
        aes_ige_encrypt(&plaintext, &key, &iv).unwrap()
    };
    let decrypt = |encrypted: &[u8]| {
        decrypt_server_dh_inner_data(encrypted, &new_nonce, &nonce, &server_nonce)
    };

    assert_eq!(
        decrypt(&[]).unwrap_err(),
        HandshakeError::InvalidServerDhCiphertext
    );
    assert_eq!(
        decrypt(&[0_u8; 15]).unwrap_err(),
        HandshakeError::InvalidServerDhCiphertext
    );

    let exact = encode_inner(
        SERVER_DH_INNER_DATA_CONSTRUCTOR,
        nonce,
        server_nonce,
        3,
        &dh_prime,
        &g_a,
        -123_456_789,
    );
    let parsed = decrypt(&encrypt_inner(&exact, 0, false)).unwrap();
    assert_eq!(parsed.server_time, -123_456_789);

    let unknown = encode_inner(
        0x1122_3344,
        nonce,
        server_nonce,
        3,
        &dh_prime,
        &g_a,
        1_720_000_000,
    );
    assert_eq!(
        decrypt(&encrypt_inner(&unknown, 0, false)).unwrap_err(),
        HandshakeError::UnexpectedServerDhInnerConstructor(0x1122_3344)
    );

    for cut in 0..exact.len() {
        assert!(
            decrypt(&encrypt_inner(&exact[..cut], 0, false)).is_err(),
            "truncation cut {cut} unexpectedly accepted"
        );
    }

    let wrong_nonce = encode_inner(
        SERVER_DH_INNER_DATA_CONSTRUCTOR,
        [0x13; 16],
        server_nonce,
        3,
        &dh_prime,
        &g_a,
        1_720_000_000,
    );
    assert_eq!(
        decrypt(&encrypt_inner(&wrong_nonce, 0, false)).unwrap_err(),
        HandshakeError::NonceMismatch
    );

    let wrong_server_nonce = encode_inner(
        SERVER_DH_INNER_DATA_CONSTRUCTOR,
        nonce,
        [0x35; 16],
        3,
        &dh_prime,
        &g_a,
        1_720_000_000,
    );
    assert_eq!(
        decrypt(&encrypt_inner(&wrong_server_nonce, 0, false)).unwrap_err(),
        HandshakeError::ServerNonceMismatch
    );

    assert_eq!(
        decrypt(&encrypt_inner(&exact, 0, true)).unwrap_err(),
        HandshakeError::ServerDhHashMismatch
    );
    assert!(matches!(
        decrypt(&encrypt_inner(&exact, 16, false)).unwrap_err(),
        HandshakeError::InvalidServerDhPadding(length) if length > 15
    ));

    let bad_generator = encode_inner(
        SERVER_DH_INNER_DATA_CONSTRUCTOR,
        nonce,
        server_nonce,
        8,
        &dh_prime,
        &g_a,
        1_720_000_000,
    );
    assert_eq!(
        decrypt(&encrypt_inner(&bad_generator, 0, false)).unwrap_err(),
        HandshakeError::InvalidServerDhParameters
    );

    let short_prime = vec![0x80; 255];
    let bad_prime_length = encode_inner(
        SERVER_DH_INNER_DATA_CONSTRUCTOR,
        nonce,
        server_nonce,
        3,
        &short_prime,
        &g_a,
        1_720_000_000,
    );
    assert_eq!(
        decrypt(&encrypt_inner(&bad_prime_length, 0, false)).unwrap_err(),
        HandshakeError::InvalidServerDhParameters
    );

    let untrusted_prime = vec![0x80; 256];
    let bad_prime_trust = encode_inner(
        SERVER_DH_INNER_DATA_CONSTRUCTOR,
        nonce,
        server_nonce,
        3,
        &untrusted_prime,
        &g_a,
        1_720_000_000,
    );
    assert_eq!(
        decrypt(&encrypt_inner(&bad_prime_trust, 0, false)).unwrap_err(),
        HandshakeError::UntrustedDhPrime
    );

    let empty_g_a = encode_inner(
        SERVER_DH_INNER_DATA_CONSTRUCTOR,
        nonce,
        server_nonce,
        3,
        &dh_prime,
        &[],
        1_720_000_000,
    );
    assert_eq!(
        decrypt(&encrypt_inner(&empty_g_a, 0, false)).unwrap_err(),
        HandshakeError::InvalidServerDhParameters
    );

    let oversized_g_a = vec![0x80; 257];
    let bad_g_a_length = encode_inner(
        SERVER_DH_INNER_DATA_CONSTRUCTOR,
        nonce,
        server_nonce,
        3,
        &dh_prime,
        &oversized_g_a,
        1_720_000_000,
    );
    assert_eq!(
        decrypt(&encrypt_inner(&bad_g_a_length, 0, false)).unwrap_err(),
        HandshakeError::InvalidServerDhParameters
    );

    let unsafe_g_a = [1_u8];
    let bad_g_a_value = encode_inner(
        SERVER_DH_INNER_DATA_CONSTRUCTOR,
        nonce,
        server_nonce,
        3,
        &dh_prime,
        &unsafe_g_a,
        1_720_000_000,
    );
    assert_eq!(
        decrypt(&encrypt_inner(&bad_g_a_value, 0, false)).unwrap_err(),
        HandshakeError::InvalidDhPublicValue
    );
}

#[test]
fn rejected_server_dh_inner_data_does_not_advance_handshake_state() {
    let nonce = [0x11; 16];
    let server_nonce = [0x22; 16];
    let new_nonce = [0x56; 32];
    let fingerprint = telegram_server_rsa_key(false)
        .unwrap()
        .fingerprint()
        .unwrap();

    let mut res_pq = TlWriter::new();
    res_pq.write_u32(RES_PQ_CONSTRUCTOR);
    res_pq.write_i128_bytes(&nonce);
    res_pq.write_i128_bytes(&server_nonce);
    res_pq.write_bytes(&[0x04, 0x2a, 0x39]).unwrap();
    res_pq.write_vector_length(1).unwrap();
    res_pq.write_u64(fingerprint);

    let mut handshake = AuthKeyHandshake::new(nonce);
    handshake.begin(4).unwrap();
    handshake
        .receive_res_pq(PlaintextEnvelope {
            message_id: 5,
            body: res_pq.into_bytes(),
        })
        .unwrap();
    handshake
        .prepare_req_dh_params_with_random(8, 2, false, |output| {
            output.fill(0x56);
            Ok(())
        })
        .unwrap();
    assert_eq!(handshake.state(), &AuthKeyHandshakeState::AwaitingServerDh);

    let dh_prime = hex::decode(KNOWN_DH_PRIME_HEX).unwrap();
    let mut g_a = vec![0_u8; 256];
    g_a[0] = 0x80;
    let mut invalid_inner = TlWriter::new();
    invalid_inner.write_u32(0x1122_3344);
    invalid_inner.write_i128_bytes(&nonce);
    invalid_inner.write_i128_bytes(&server_nonce);
    invalid_inner.write_i32(3);
    invalid_inner.write_bytes(&dh_prime).unwrap();
    invalid_inner.write_bytes(&g_a).unwrap();
    invalid_inner.write_i32(1_720_000_000);
    let invalid_inner = invalid_inner.into_bytes();

    let mut plaintext = Sha1::digest(&invalid_inner).to_vec();
    plaintext.extend_from_slice(&invalid_inner);
    while plaintext.len() % 16 != 0 {
        plaintext.push(0xa5);
    }
    let (key, iv) = derive_tmp_aes_key_iv(&new_nonce, &server_nonce);
    let encrypted_answer = aes_ige_encrypt(&plaintext, &key, &iv).unwrap();

    let mut outer = TlWriter::new();
    outer.write_u32(SERVER_DH_PARAMS_OK_CONSTRUCTOR);
    outer.write_i128_bytes(&nonce);
    outer.write_i128_bytes(&server_nonce);
    outer.write_bytes(&encrypted_answer).unwrap();

    assert_eq!(
        handshake
            .receive_server_dh_with_random(
                PlaintextEnvelope {
                    message_id: 9,
                    body: outer.into_bytes(),
                },
                12,
                |output| {
                    output.fill(0x44);
                    Ok(())
                },
            )
            .unwrap_err(),
        HandshakeError::UnexpectedServerDhInnerConstructor(0x1122_3344)
    );
    assert_eq!(handshake.state(), &AuthKeyHandshakeState::AwaitingServerDh);
}

#[test]
fn client_dh_inner_data_builder_serializes_fixed_u_object_exactly() {
    let nonce = [
        0x00, 0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77,
        0x88, 0x99, 0xaa, 0xbb, 0xcc, 0xdd, 0xee, 0xff,
    ];
    let server_nonce = [
        0xff, 0xee, 0xdd, 0xcc, 0xbb, 0xaa, 0x99, 0x88,
        0x77, 0x66, 0x55, 0x44, 0x33, 0x22, 0x11, 0x00,
    ];
    let retry_id = 0x0102_0304_0506_0708_u64;
    let g_b = [0x10, 0x20, 0x30, 0x40, 0x50];
    let encoded =
        build_client_dh_inner_data(&nonce, &server_nonce, retry_id, &g_b).unwrap();

    let mut expected = Vec::new();
    expected.extend_from_slice(&CLIENT_DH_INNER_DATA_CONSTRUCTOR.to_le_bytes());
    expected.extend_from_slice(&nonce);
    expected.extend_from_slice(&server_nonce);
    expected.extend_from_slice(&retry_id.to_le_bytes());
    expected.push(g_b.len() as u8);
    expected.extend_from_slice(&g_b);
    expected.extend_from_slice(&[0_u8; 2]);
    assert_eq!(encoded, expected);

    let mut reader = TlReader::new(&encoded);
    assert_eq!(
        reader.read_u32().unwrap(),
        CLIENT_DH_INNER_DATA_CONSTRUCTOR
    );
    assert_eq!(reader.read_i128_bytes().unwrap(), nonce);
    assert_eq!(reader.read_i128_bytes().unwrap(), server_nonce);
    assert_eq!(reader.read_u64().unwrap(), retry_id);
    assert_eq!(reader.read_bytes().unwrap(), g_b);
    assert!(reader.is_finished());
}

#[test]
fn set_client_dh_params_builder_serializes_fixed_u_request_exactly() {
    let nonce = [
        0x00, 0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77,
        0x88, 0x99, 0xaa, 0xbb, 0xcc, 0xdd, 0xee, 0xff,
    ];
    let server_nonce = [
        0xff, 0xee, 0xdd, 0xcc, 0xbb, 0xaa, 0x99, 0x88,
        0x77, 0x66, 0x55, 0x44, 0x33, 0x22, 0x11, 0x00,
    ];
    let encrypted_data = [0x10, 0x20, 0x30, 0x40, 0x50];
    let encoded =
        build_set_client_dh_params(&nonce, &server_nonce, &encrypted_data).unwrap();

    let mut expected = Vec::new();
    expected.extend_from_slice(&SET_CLIENT_DH_PARAMS_CONSTRUCTOR.to_le_bytes());
    expected.extend_from_slice(&nonce);
    expected.extend_from_slice(&server_nonce);
    expected.push(encrypted_data.len() as u8);
    expected.extend_from_slice(&encrypted_data);
    expected.extend_from_slice(&[0_u8; 2]);
    assert_eq!(encoded, expected);

    let mut reader = TlReader::new(&encoded);
    assert_eq!(
        reader.read_u32().unwrap(),
        SET_CLIENT_DH_PARAMS_CONSTRUCTOR
    );
    assert_eq!(reader.read_i128_bytes().unwrap(), nonce);
    assert_eq!(reader.read_i128_bytes().unwrap(), server_nonce);
    assert_eq!(reader.read_bytes().unwrap(), encrypted_data);
    assert!(reader.is_finished());
}

#[test]
fn client_dh_derives_auth_key_and_authenticates_dh_gen_ok() {
    let nonce = [0x12; 16];
    let server_nonce = [0x34; 16];
    let new_nonce = [0x56; 32];
    let mut g_a = vec![0_u8; 256];
    g_a[0] = 0x80;
    let server = ServerDhInnerData {
        nonce,
        server_nonce,
        generator: 3,
        dh_prime: hex::decode(KNOWN_DH_PRIME_HEX).unwrap(),
        g_a,
        server_time: 1_720_000_000,
    };
    let fill = |output: &mut [u8]| {
        for (index, byte) in output.iter_mut().enumerate() {
            *byte = (index as u8).wrapping_mul(29).wrapping_add(7);
        }
        Ok(())
    };
    let prepared = prepare_client_dh(&server, &new_nonce, 0, fill).unwrap();
    assert_ne!(prepared.auth_key.id(), 0);
    assert_eq!(prepared.server_salt, i64::from_le_bytes([0x62; 8]));
    let mut request = TlReader::new(&prepared.request_body);
    assert_eq!(
        request.read_u32().unwrap(),
        SET_CLIENT_DH_PARAMS_CONSTRUCTOR
    );
    assert_eq!(request.read_i128_bytes().unwrap(), nonce);
    assert_eq!(request.read_i128_bytes().unwrap(), server_nonce);
    let encrypted_data = request.read_bytes().unwrap().to_vec();
    assert!(!encrypted_data.is_empty());
    assert!(request.is_finished());
    assert_eq!(
        prepared.request_body,
        build_set_client_dh_params(&nonce, &server_nonce, &encrypted_data).unwrap()
    );

    let mut material = Vec::from(new_nonce);
    material.push(1);
    material.extend_from_slice(&prepared.auth_key_aux_hash.to_le_bytes());
    let digest = Sha1::digest(&material);
    let expected_hash: [u8; 16] = digest[4..20].try_into().unwrap();
    let mut answer = TlWriter::new();
    answer.write_u32(DH_GEN_OK_CONSTRUCTOR);
    answer.write_i128_bytes(&nonce);
    answer.write_i128_bytes(&server_nonce);
    answer.write_i128_bytes(&expected_hash);
    assert_eq!(
        parse_dh_gen_result(
            &answer.into_bytes(),
            &nonce,
            &server_nonce,
            &new_nonce,
            prepared.auth_key_aux_hash,
        )
        .unwrap(),
        DhGenResult::Ok
    );

    let mut invalid = TlWriter::new();
    invalid.write_u32(DH_GEN_OK_CONSTRUCTOR);
    invalid.write_i128_bytes(&nonce);
    invalid.write_i128_bytes(&server_nonce);
    invalid.write_i128_bytes(&[0; 16]);
    assert_eq!(
        parse_dh_gen_result(
            &invalid.into_bytes(),
            &nonce,
            &server_nonce,
            &new_nonce,
            prepared.auth_key_aux_hash,
        )
        .unwrap_err(),
        HandshakeError::DhGenHashMismatch
    );
}

#[test]
fn dh_gen_result_parser_is_exact_for_all_fixed_u_variants() {
    let nonce = [0x21; 16];
    let server_nonce = [0x43; 16];
    let new_nonce = [0x65; 32];
    let auth_key_aux_hash = 0x8877_6655_4433_2211_u64;

    let encode = |constructor: u32, tag: u8, body_nonce: [u8; 16], body_server_nonce: [u8; 16]| {
        let mut material = Vec::from(new_nonce);
        material.push(tag);
        material.extend_from_slice(&auth_key_aux_hash.to_le_bytes());
        let digest = Sha1::digest(&material);
        let expected_hash: [u8; 16] = digest[4..20].try_into().unwrap();

        let mut writer = TlWriter::new();
        writer.write_u32(constructor);
        writer.write_i128_bytes(&body_nonce);
        writer.write_i128_bytes(&body_server_nonce);
        writer.write_i128_bytes(&expected_hash);
        writer.into_bytes()
    };

    for (constructor, tag, expected) in [
        (DH_GEN_OK_CONSTRUCTOR, 1_u8, DhGenResult::Ok),
        (DH_GEN_RETRY_CONSTRUCTOR, 2_u8, DhGenResult::Retry),
        (DH_GEN_FAIL_CONSTRUCTOR, 3_u8, DhGenResult::Fail),
    ] {
        let body = encode(constructor, tag, nonce, server_nonce);
        assert_eq!(
            parse_dh_gen_result(
                &body,
                &nonce,
                &server_nonce,
                &new_nonce,
                auth_key_aux_hash,
            )
            .unwrap(),
            expected
        );

        for cut in 0..body.len() {
            assert!(
                parse_dh_gen_result(
                    &body[..cut],
                    &nonce,
                    &server_nonce,
                    &new_nonce,
                    auth_key_aux_hash,
                )
                .is_err(),
                "truncation cut {cut} unexpectedly accepted for constructor 0x{constructor:08x}"
            );
        }

        let mut trailing = body.clone();
        trailing.push(0x99);
        assert_eq!(
            parse_dh_gen_result(
                &trailing,
                &nonce,
                &server_nonce,
                &new_nonce,
                auth_key_aux_hash,
            )
            .unwrap_err(),
            HandshakeError::TrailingBodyBytes(1)
        );
    }

    assert_eq!(
        parse_dh_gen_result(
            &0x1122_3344_u32.to_le_bytes(),
            &nonce,
            &server_nonce,
            &new_nonce,
            auth_key_aux_hash,
        )
        .unwrap_err(),
        HandshakeError::UnexpectedDhGenConstructor(0x1122_3344)
    );

    let wrong_nonce = encode(DH_GEN_OK_CONSTRUCTOR, 1, [0x20; 16], server_nonce);
    assert_eq!(
        parse_dh_gen_result(
            &wrong_nonce,
            &nonce,
            &server_nonce,
            &new_nonce,
            auth_key_aux_hash,
        )
        .unwrap_err(),
        HandshakeError::NonceMismatch
    );

    let wrong_server_nonce = encode(DH_GEN_RETRY_CONSTRUCTOR, 2, nonce, [0x42; 16]);
    assert_eq!(
        parse_dh_gen_result(
            &wrong_server_nonce,
            &nonce,
            &server_nonce,
            &new_nonce,
            auth_key_aux_hash,
        )
        .unwrap_err(),
        HandshakeError::ServerNonceMismatch
    );

    for (constructor, wrong_tag) in [
        (DH_GEN_OK_CONSTRUCTOR, 2_u8),
        (DH_GEN_RETRY_CONSTRUCTOR, 3_u8),
        (DH_GEN_FAIL_CONSTRUCTOR, 1_u8),
    ] {
        let body = encode(constructor, wrong_tag, nonce, server_nonce);
        assert_eq!(
            parse_dh_gen_result(
                &body,
                &nonce,
                &server_nonce,
                &new_nonce,
                auth_key_aux_hash,
            )
            .unwrap_err(),
            HandshakeError::DhGenHashMismatch
        );
    }
}

#[test]
fn dc_directory_filters_and_prioritizes_dynamic_main_media_and_ipv6_endpoints() {
    let endpoints = vec![
        DcEndpoint::new(
            2,
            "149.154.167.40".parse().unwrap(),
            443,
            false,
            false,
            false,
            true,
            false,
            None,
        )
        .unwrap(),
        DcEndpoint::new(
            2,
            "2001:67c:4e8:f002::a".parse().unwrap(),
            443,
            false,
            false,
            false,
            false,
            false,
            None,
        )
        .unwrap(),
        DcEndpoint::new(
            4,
            "149.154.167.92".parse().unwrap(),
            443,
            false,
            false,
            false,
            false,
            false,
            None,
        )
        .unwrap(),
        DcEndpoint::new(
            4,
            "149.154.167.93".parse().unwrap(),
            443,
            true,
            false,
            false,
            false,
            false,
            None,
        )
        .unwrap(),
        DcEndpoint::new(
            4,
            "149.154.167.94".parse().unwrap(),
            443,
            false,
            false,
            true,
            false,
            false,
            None,
        )
        .unwrap(),
    ];
    let directory = DcDirectory::new(2, false, endpoints).unwrap();

    let ipv6_first = directory.candidates(2, DcPurpose::Main, true).unwrap();
    assert!(ipv6_first[0].ip_address.is_ipv6());
    assert_eq!(
        directory
            .candidates(4, DcPurpose::Main, false)
            .unwrap()
            .len(),
        1
    );

    let media = directory.candidates(4, DcPurpose::Media, false).unwrap();
    assert_eq!(media.len(), 2);
    assert!(media[0].media_only);

    let cdn = directory.candidates(4, DcPurpose::Cdn, false).unwrap();
    assert_eq!(cdn.len(), 1);
    assert!(cdn[0].cdn);
}

#[test]
fn rpc_errors_are_classified_without_accepting_malformed_redirects_or_waits() {
    assert_eq!(
        classify_rpc_error(303, "PHONE_MIGRATE_4"),
        RpcErrorAction::Migrate(MigrationDirective {
            kind: MigrationKind::Phone,
            dc_id: 4,
        })
    );
    assert_eq!(
        classify_rpc_error(303, "FILE_MIGRATE_5"),
        RpcErrorAction::Migrate(MigrationDirective {
            kind: MigrationKind::File,
            dc_id: 5,
        })
    );
    assert_eq!(
        classify_rpc_error(420, "FLOOD_WAIT_17"),
        RpcErrorAction::Wait {
            seconds: 17,
            premium: false,
        }
    );
    assert_eq!(
        classify_rpc_error(420, "FLOOD_PREMIUM_WAIT_3"),
        RpcErrorAction::Wait {
            seconds: 3,
            premium: true,
        }
    );
    assert_eq!(
        classify_rpc_error(401, "AUTH_KEY_DUPLICATED"),
        RpcErrorAction::Reauthorize
    );
    assert_eq!(
        classify_rpc_error(500, "INTERNAL"),
        RpcErrorAction::RetryTransient
    );
    assert_eq!(
        classify_rpc_error(303, "PHONE_MIGRATE_0"),
        RpcErrorAction::Fail
    );
    assert_eq!(
        classify_rpc_error(303, "PHONE_MIGRATE_4_EXTRA"),
        RpcErrorAction::Fail
    );
    assert_eq!(
        classify_rpc_error(420, "FLOOD_WAIT_-1"),
        RpcErrorAction::Fail
    );
}

#[test]
fn dc_routing_switches_main_for_account_migrations_but_keeps_file_routes_isolated() {
    let directory = DcDirectory::new(
        2,
        false,
        vec![
            DcEndpoint::new(
                2,
                "149.154.167.40".parse().unwrap(),
                443,
                false,
                false,
                false,
                true,
                false,
                None,
            )
            .unwrap(),
            DcEndpoint::new(
                4,
                "149.154.167.92".parse().unwrap(),
                443,
                false,
                false,
                false,
                false,
                false,
                None,
            )
            .unwrap(),
            DcEndpoint::new(
                4,
                "149.154.167.93".parse().unwrap(),
                443,
                true,
                false,
                false,
                false,
                false,
                None,
            )
            .unwrap(),
        ],
    )
    .unwrap();

    assert_eq!(
        directory.route_rpc_error(303, "USER_MIGRATE_4").unwrap(),
        DcRoute::SwitchMain {
            dc_id: 4,
            reason: MigrationKind::User,
        }
    );
    assert_eq!(
        directory.route_rpc_error(303, "FILE_MIGRATE_4").unwrap(),
        DcRoute::RetryOn {
            dc_id: 4,
            purpose: DcPurpose::Media,
        }
    );
    assert_eq!(
        directory
            .route_rpc_error(303, "NETWORK_MIGRATE_5")
            .unwrap_err(),
        DcError::MissingEndpoint {
            dc_id: 5,
            purpose: DcPurpose::Main,
        }
    );
}


#[test]
fn fixed_u_auth_lifecycle_builders_preserve_constructor_and_field_order() {
    let resend = build_auth_resend_code("+15551234567", "phone-hash", Some("timeout")).unwrap();
    let mut reader = TlReader::new(&resend);
    assert_eq!(reader.read_u32().unwrap(), 0xcae4_7523);
    assert_eq!(reader.read_i32().unwrap(), 1);
    assert_eq!(reader.read_string().unwrap(), "+15551234567");
    assert_eq!(reader.read_string().unwrap(), "phone-hash");
    assert_eq!(reader.read_string().unwrap(), "timeout");
    assert!(reader.is_finished());

    let cancel = build_auth_cancel_code("+15551234567", "phone-hash").unwrap();
    let mut reader = TlReader::new(&cancel);
    assert_eq!(reader.read_u32().unwrap(), 0x1f04_0578);
    assert_eq!(reader.read_string().unwrap(), "+15551234567");
    assert_eq!(reader.read_string().unwrap(), "phone-hash");
    assert!(reader.is_finished());

    assert_eq!(
        u32::from_le_bytes(build_auth_request_password_recovery().try_into().unwrap()),
        0xd897_bc66
    );
    let recovery = build_auth_check_recovery_password("123456").unwrap();
    let mut reader = TlReader::new(&recovery);
    assert_eq!(reader.read_u32().unwrap(), 0x0d36_bf79);
    assert_eq!(reader.read_string().unwrap(), "123456");
    assert!(reader.is_finished());

    let send_email =
        build_account_send_verify_email_code("+15551234567", "phone-hash", "a@example.test")
            .unwrap();
    let mut reader = TlReader::new(&send_email);
    assert_eq!(reader.read_u32().unwrap(), 0x98e0_37bb);
    assert_eq!(reader.read_u32().unwrap(), 0x4345_be73);
    assert_eq!(reader.read_string().unwrap(), "+15551234567");
    assert_eq!(reader.read_string().unwrap(), "phone-hash");
    assert_eq!(reader.read_string().unwrap(), "a@example.test");
    assert!(reader.is_finished());

    let verify = build_account_verify_email("+15551234567", "phone-hash", "654321").unwrap();
    let mut reader = TlReader::new(&verify);
    assert_eq!(reader.read_u32().unwrap(), 0x032d_a4cf);
    assert_eq!(reader.read_u32().unwrap(), 0x4345_be73);
    assert_eq!(reader.read_string().unwrap(), "+15551234567");
    assert_eq!(reader.read_string().unwrap(), "phone-hash");
    assert_eq!(reader.read_u32().unwrap(), 0x922e_55a9);
    assert_eq!(reader.read_string().unwrap(), "654321");
    assert!(reader.is_finished());
}

#[test]
fn fixed_u_auth_lifecycle_parsers_keep_recovery_email_and_login_code_state() {
    let mut recovery = TlWriter::new();
    recovery.write_u32(0x1379_48a5);
    recovery.write_string("m***@example.test").unwrap();
    assert_eq!(
        parse_auth_password_recovery(recovery.as_slice())
            .unwrap()
            .email_pattern,
        "m***@example.test"
    );

    let mut sent_email = TlWriter::new();
    sent_email.write_u32(0x811f_854f);
    sent_email.write_string("m***@example.test").unwrap();
    sent_email.write_i32(6);
    let sent = parse_account_sent_email_code(sent_email.as_slice()).unwrap();
    assert_eq!(sent.email_pattern, "m***@example.test");
    assert_eq!(sent.length, 6);

    let mut bool_true = TlWriter::new();
    bool_true.write_bool(true);
    assert!(parse_bool(bool_true.as_slice()).unwrap());

    let mut verified = TlWriter::new();
    verified.write_u32(ACCOUNT_EMAIL_VERIFIED_LOGIN_CONSTRUCTOR);
    verified.write_string("member@example.test").unwrap();
    verified.write_u32(AUTH_SENT_CODE_CONSTRUCTOR);
    verified.write_i32(0);
    verified.write_u32(0xc000_bba2);
    verified.write_i32(5);
    verified.write_string("next-phone-hash").unwrap();
    match parse_account_email_verified(verified.as_slice()).unwrap() {
        EmailVerifiedResult::LoginCode { email, code } => {
            assert_eq!(email, "member@example.test");
            assert_eq!(code.phone_code_hash, "next-phone-hash");
            assert_eq!(code.delivery, SentCodeDelivery::Sms { length: 5 });
        }
        other => panic!("unexpected email verification result: {other:?}"),
    }
}
#[test]
fn destroy_auth_key_result_parses_all_fixed_u_variants_exactly() {
    for (constructor, expected) in [
        (DESTROY_AUTH_KEY_OK_CONSTRUCTOR, DestroyAuthKeyResult::Ok),
        (DESTROY_AUTH_KEY_NONE_CONSTRUCTOR, DestroyAuthKeyResult::None),
        (DESTROY_AUTH_KEY_FAIL_CONSTRUCTOR, DestroyAuthKeyResult::Fail),
    ] {
        assert_eq!(
            parse_destroy_auth_key_result(&constructor.to_le_bytes())
                .expect("parse DestroyAuthKeyRes variant"),
            expected
        );
    }
}

#[test]
fn destroy_auth_key_result_rejects_unknown_truncation_and_trailing_bytes() {
    let unknown = 0_u32.to_le_bytes();
    assert_eq!(
        parse_destroy_auth_key_result(&unknown).unwrap_err(),
        ApiRequestError::UnexpectedDestroyAuthKeyConstructor(0)
    );

    let exact = DESTROY_AUTH_KEY_OK_CONSTRUCTOR.to_le_bytes();
    for cut in 0..exact.len() {
        assert!(parse_destroy_auth_key_result(&exact[..cut]).is_err());
    }

    let mut trailing = exact.to_vec();
    trailing.extend_from_slice(&0_u32.to_le_bytes());
    assert!(parse_destroy_auth_key_result(&trailing).is_err());
}

