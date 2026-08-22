use super::*;
use pretty_assertions::assert_eq;

#[test]
fn migration_contains_the_authoritative_commerce_tables() {
    assert_eq!(validate_platform_schema(PLATFORM_SCHEMA_V1), Ok(()));
}

#[test]
fn fabushi_pay_migration_contains_payment_and_settlement_invariants() {
    assert_eq!(validate_fabushi_pay_schema(FABUSHI_PAY_SCHEMA_V7), Ok(()));
    assert!(FABUSHI_PAY_SCHEMA_V7.contains("UNIQUE (user_id, idempotency_key)"));
    assert!(FABUSHI_PAY_SCHEMA_V7.contains("PRIMARY KEY (provider, event_id)"));
    assert!(FABUSHI_PAY_SCHEMA_V7.contains("idempotency_key TEXT NOT NULL UNIQUE"));
    assert!(FABUSHI_PAY_SCHEMA_V7.contains("provider_product_refs_json"));
    assert!(FABUSHI_PAY_SCHEMA_V7.contains("developer_settlement_releases_payment_idx"));
    assert!(
        !FABUSHI_PAY_SCHEMA_V7
            .contains("payment_id TEXT NOT NULL UNIQUE REFERENCES payment_intents")
    );
    assert!(!FABUSHI_PAY_SCHEMA_V7.contains("amount REAL"));
}

#[test]
fn listener_relay_migration_contains_durable_registration_and_event_tables() {
    assert_eq!(
        validate_listener_relay_schema(LISTENER_RELAY_SCHEMA_V5),
        Ok(())
    );
    assert!(LISTENER_RELAY_SCHEMA_V5.contains("acknowledged_at"));
    assert!(LISTENER_RELAY_SCHEMA_V5.contains("listener_events_pending_idx"));
}

#[test]
fn remote_computer_migration_keeps_control_plane_separate_from_desktop_data() {
    assert_eq!(
        validate_remote_computer_schema(REMOTE_COMPUTER_SCHEMA_V6),
        Ok(())
    );
    assert!(REMOTE_COMPUTER_SCHEMA_V6.contains("remote_computer_sessions"));
    assert!(REMOTE_COMPUTER_SCHEMA_V6.contains("remote_computer_signals"));
    assert!(REMOTE_COMPUTER_SCHEMA_V6.contains("device_secret_hash TEXT NOT NULL"));
    assert!(!REMOTE_COMPUTER_SCHEMA_V6.contains("screenshot_data"));
    assert!(!REMOTE_COMPUTER_SCHEMA_V6.contains("input_payload"));
}

#[test]
fn account_auth_migration_contains_rotating_session_state() {
    assert_eq!(validate_account_auth_schema(ACCOUNT_AUTH_SCHEMA_V2), Ok(()));
}

#[test]
fn oauth_schema_separates_provider_identity_from_user_profile() {
    assert_eq!(
        validate_account_oauth_schema(ACCOUNT_OAUTH_SCHEMA_V3),
        Ok(())
    );
    assert!(ACCOUNT_OAUTH_SCHEMA_V3.contains("UNIQUE(issuer, subject)"));
    assert!(!ACCOUNT_OAUTH_SCHEMA_V3.contains("UNIQUE(email)"));
}

#[test]
fn registration_schema_hashes_codes_and_binds_them_to_browser_attempts() {
    assert_eq!(
        validate_account_registration_schema(ACCOUNT_REGISTRATION_SCHEMA_V5),
        Ok(())
    );
    assert!(ACCOUNT_REGISTRATION_SCHEMA_V5.contains("code_hash TEXT NOT NULL"));
    assert!(ACCOUNT_REGISTRATION_SCHEMA_V5.contains("attempt_id TEXT NOT NULL"));
    assert!(ACCOUNT_REGISTRATION_SCHEMA_V5.contains("failed_attempts INTEGER NOT NULL"));
    assert!(!ACCOUNT_REGISTRATION_SCHEMA_V5.contains("code TEXT NOT NULL"));
}

#[test]
fn oauth_attempt_terminal_states_include_failed() {
    assert!(ACCOUNT_OAUTH_STATUS_SCHEMA_V4.contains("'cancelled', 'failed'"));
    assert!(ACCOUNT_OAUTH_STATUS_SCHEMA_V4.contains("FROM account_oauth_attempts"));
    assert!(ACCOUNT_OAUTH_STATUS_SCHEMA_V4.contains("RENAME TO account_oauth_attempts"));
}

#[test]
fn validation_rejects_floating_point_money() {
    let schema = PLATFORM_SCHEMA_V1.replace("amount INTEGER", "amount REAL");
    assert_eq!(
        validate_platform_schema(&schema),
        Err(SchemaError::FloatingPointAmount)
    );

    let schema = FABUSHI_PAY_SCHEMA_V7.replace("amount INTEGER", "amount REAL");
    assert_eq!(
        validate_fabushi_pay_schema(&schema),
        Err(SchemaError::FloatingPointAmount)
    );
}
