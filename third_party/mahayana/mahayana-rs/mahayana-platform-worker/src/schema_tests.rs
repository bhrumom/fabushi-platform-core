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
    assert!(REMOTE_COMPUTER_CLIENT_TOKEN_SCHEMA_V14.contains("client_token_hash TEXT"));
    assert!(REMOTE_COMPUTER_CLIENT_TOKEN_SCHEMA_V14.contains("SET state = 'closed'"));
    assert!(REMOTE_COMPUTER_CLIENT_TOKEN_SCHEMA_V14.contains("SET revoked_at = COALESCE"));
    assert!(!REMOTE_COMPUTER_CLIENT_TOKEN_SCHEMA_V14.contains("client_token TEXT"));
}

#[test]
fn marketplace_account_install_migration_is_account_scoped_and_manifest_projected() {
    assert_eq!(
        validate_marketplace_account_install_schema(MARKETPLACE_ACCOUNT_INSTALL_SCHEMA_V18),
        Ok(())
    );
    for required in [
        "PRIMARY KEY (account_user_id, plugin_id)",
        "REFERENCES marketplace_plugins(plugin_id) ON DELETE RESTRICT",
        "global-dharma-bot",
        "open-miniapp",
        "miniapp:global-dharma",
    ] {
        assert!(
            MARKETPLACE_ACCOUNT_INSTALL_SCHEMA_V18.contains(required),
            "missing {required}"
        );
    }
    assert!(!MARKETPLACE_ACCOUNT_INSTALL_SCHEMA_V18.contains("access_token"));
    assert!(!MARKETPLACE_ACCOUNT_INSTALL_SCHEMA_V18.contains("refresh_token"));
}

#[test]
fn marketplace_route_projection_declares_and_resolves_official_webmcp_status() {
    for required in [
        "remote-mcp",
        "mcp-http",
        "https://api.ombhrum.com/api/mcp/apps/global-dharma",
        "naturalLanguageHints",
        "validate_config",
        "deploy_latest",
    ] {
        assert!(
            MARKETPLACE_ROUTE_PROJECTION_SCHEMA_V19.contains(required),
            "missing {required}"
        );
    }
    let projection = serde_json::json!({
        "bot": {"id": "global-dharma-bot"},
        "surfaces": [{"id": "remote-mcp", "kind": "mcp-http", "url": "https://api.ombhrum.com/api/mcp/apps/global-dharma"}],
        "commands": [{
            "name": "status",
            "description": "Read status",
            "surfaceId": "remote-mcp",
            "tool": "status",
            "approval": "none",
            "aliases": ["状态"],
            "naturalLanguageHints": ["show status"]
        }]
    });
    let natural = crate::marketplace_route::route_marketplace_input(
        "global-dharma",
        &projection,
        "please show status now",
    )
    .expect("natural route");
    assert_eq!(natural["execution"]["kind"], "mcp-http");
    assert_eq!(natural["execution"]["tool"], "status");
    assert_eq!(natural["requiresApproval"], false);
    assert_eq!(natural["arguments"]["input"], "please show status now");

    let slash = crate::marketplace_route::route_marketplace_input(
        "global-dharma",
        &projection,
        "/global-dharma:status {\"detail\":true}",
    )
    .expect("slash route");
    assert_eq!(slash["command"]["slash"], "/global-dharma:status");
    assert_eq!(slash["arguments"]["detail"], true);
}

#[test]
fn remote_computer_inventory_migration_is_additive_and_secret_free() {
    for required in [
        "provider TEXT NOT NULL DEFAULT 'fabushi-webrtc'",
        "platform TEXT NOT NULL DEFAULT 'unknown'",
        "app_version TEXT NOT NULL DEFAULT 'unknown'",
        "capabilities_json TEXT NOT NULL DEFAULT '[]'",
        "rustdesk-sidecar",
        "remote_computers_inventory_idx",
    ] {
        assert!(
            REMOTE_COMPUTER_INVENTORY_SCHEMA_V15.contains(required),
            "missing {required}"
        );
    }
    assert!(!REMOTE_COMPUTER_INVENTORY_SCHEMA_V15.contains("device_secret TEXT"));
    assert!(!REMOTE_COMPUTER_INVENTORY_SCHEMA_V15.contains("screenshot_data"));
    assert!(!REMOTE_COMPUTER_INVENTORY_SCHEMA_V15.contains("input_payload"));
}

#[test]
fn ci_runner_auth_is_exact_workflow_and_linked_account_scoped() {
    for required in [
        "token.actions.githubusercontent.com",
        "bhrumom/fabushi",
        "1037709914",
        "281146136",
        "interactive-runner-mcp.yml@refs/heads/main",
        "refs/heads/main",
        "ref_protected",
        "workflow_dispatch",
        "github-hosted",
        "account_identities",
        "provider = 'github'",
        "CI_OIDC_MAX_AGE_SECONDS",
    ] {
        assert!(
            CI_RUNNER_AUTH_SOURCE_V1.contains(required),
            "missing {required}"
        );
    }
    assert!(!CI_RUNNER_AUTH_SOURCE_V1.contains("TEST_ACCOUNT_TOKEN"));
    assert!(!CI_RUNNER_AUTH_SOURCE_V1.contains("refreshToken"));
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
fn principal_schema_keeps_profile_identity_and_connected_accounts_separate() {
    assert_eq!(
        validate_account_principal_schema(ACCOUNT_PRINCIPAL_SCHEMA_V6),
        Ok(())
    );
    let principal_table = ACCOUNT_PRINCIPAL_SCHEMA_V6
        .split("CREATE TABLE IF NOT EXISTS account_principals")
        .nth(1)
        .and_then(|rest| rest.split("CREATE").next())
        .expect("principal table");
    for forbidden in [
        "password",
        "apple",
        "alipay",
        "firebase",
        "membership",
        "payment",
    ] {
        assert!(
            !principal_table.to_ascii_lowercase().contains(forbidden),
            "principal row must not contain provider/product field {forbidden}"
        );
    }
    assert!(ACCOUNT_PRINCIPAL_SCHEMA_V6.contains("account_connections"));
    assert!(ACCOUNT_PRINCIPAL_SCHEMA_V6.contains("account_connection_grants"));
    assert!(ACCOUNT_PRINCIPAL_SCHEMA_V6.contains("credential_ref TEXT NOT NULL"));
    assert!(!ACCOUNT_PRINCIPAL_SCHEMA_V6.contains("access_token TEXT"));
    assert!(!ACCOUNT_PRINCIPAL_SCHEMA_V6.contains("refresh_token TEXT"));
}

#[test]
fn workspace_schema_unifies_human_and_agent_messaging() {
    assert_eq!(
        validate_workspace_messaging_schema(WORKSPACE_MESSAGING_SCHEMA_V7),
        Ok(())
    );
    assert!(
        WORKSPACE_MESSAGING_SCHEMA_V7.contains("peer_type IN ('principal', 'agent', 'system')")
    );
    assert!(WORKSPACE_MESSAGING_SCHEMA_V7.contains("UNIQUE (conversation_id, seq)"));
    assert!(WORKSPACE_MESSAGING_SCHEMA_V7.contains("UNIQUE (conversation_id, client_nonce)"));
    assert!(WORKSPACE_MESSAGING_SCHEMA_V7.contains("last_read_seq"));
    assert!(WORKSPACE_MESSAGING_SCHEMA_V7.contains("capability_policy_json"));
    assert!(
        !WORKSPACE_MESSAGING_SCHEMA_V7
            .to_ascii_lowercase()
            .contains("leaderboard")
    );
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

#[test]
fn worker_router_rejects_duplicate_developer_commerce_regressions() {
    let source = include_str!("worker_api.rs");
    let compact = source.split_whitespace().collect::<String>();
    for (method, route) in [
        ("get", "/v1/marketplace/added"),
        ("post", "/v1/marketplace/plugins/:plugin_id/add"),
        ("post", "/v1/marketplace/plugins/:plugin_id/route"),
        ("get", "/v1/developer/commerce/profile"),
        ("post", "/v1/developer/commerce/profile"),
        ("get", "/v1/developer/commerce/miniapps"),
        ("post", "/v1/developer/commerce/miniapps/:mini_app_id"),
        (
            "get",
            "/v1/developer/commerce/miniapps/:mini_app_id/products",
        ),
        (
            "post",
            "/v1/developer/commerce/miniapps/:mini_app_id/products",
        ),
        (
            "post",
            "/v1/developer/commerce/miniapps/:mini_app_id/products/:product_id",
        ),
        (
            "post",
            "/v1/developer/commerce/miniapps/:mini_app_id/products/:product_id/google/sync",
        ),
        (
            "post",
            "/v1/pay/intents/:payment_id/apple/advanced-commerce",
        ),
    ] {
        let needle = format!(".{method}_async(\"{route}\"");
        assert_eq!(
            compact.matches(&needle).count(),
            1,
            "duplicate Mahayana Worker router registration: {method} {route}"
        );
    }
}
