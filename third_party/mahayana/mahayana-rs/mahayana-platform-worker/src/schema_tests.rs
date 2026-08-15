use super::*;
use pretty_assertions::assert_eq;

#[test]
fn migration_contains_the_authoritative_commerce_tables() {
    assert_eq!(validate_platform_schema(PLATFORM_SCHEMA_V1), Ok(()));
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
fn validation_rejects_floating_point_money() {
    let schema = PLATFORM_SCHEMA_V1.replace("amount INTEGER", "amount REAL");
    assert_eq!(
        validate_platform_schema(&schema),
        Err(SchemaError::FloatingPointAmount)
    );
}
