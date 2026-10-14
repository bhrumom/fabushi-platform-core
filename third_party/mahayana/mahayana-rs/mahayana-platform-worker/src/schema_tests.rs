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
fn remote_computer_mobile_session_reconcile_contract_is_idempotent_and_secret_safe() {
    for required in [
        "mobile_request_id TEXT",
        "mobile_request_hash TEXT",
        "client_token_hash_at_create TEXT",
        "CREATE UNIQUE INDEX IF NOT EXISTS remote_computer_sessions_mobile_request_identity_idx",
        "ON remote_computer_sessions(user_id, device_id, client_id, mobile_request_id)",
        "WHERE mobile_request_id IS NOT NULL",
    ] {
        assert!(
            REMOTE_COMPUTER_MOBILE_RECONCILE_SCHEMA_V27.contains(required),
            "missing remote reconcile schema invariant: {required}"
        );
    }
    for forbidden in [
        "mobile_token TEXT",
        "client_token TEXT",
        "device_secret TEXT",
    ] {
        assert!(
            !REMOTE_COMPUTER_MOBILE_RECONCILE_SCHEMA_V27.contains(forbidden),
            "raw remote secret must not be persisted: {forbidden}"
        );
    }

    for required in [
        "request_id: String",
        "remote_mobile_request_hash(",
        "recoverable_remote_mobile_token(",
        "client_token_hash_at_create",
        "INSERT OR IGNORE INTO remote_computer_sessions",
        "load_remote_mobile_request_session(",
        "control_session_client_rotated",
        "control_session_recovery_mismatch",
        "control_session_expired",
        "remote_computer_session_reconcile",
    ] {
        assert!(
            REMOTE_COMPUTER_WORKER_SOURCE.contains(required),
            "missing remote reconcile source invariant: {required}"
        );
    }
    assert!(!REMOTE_COMPUTER_WORKER_SOURCE.contains("mobile_token, mobile_request_id"));
    assert!(
        WORKER_ROUTER_SOURCE
            .contains(".post_async(\n            \"/v1/computers/:device_id/sessions/reconcile\","),
        "mobile reconcile endpoint is not wired into the Worker router"
    );
}

#[test]
fn remote_computer_operation_journal_is_idempotent_secret_safe_and_routed() {
    for required in [
        "CREATE TABLE IF NOT EXISTS remote_computer_operations",
        "UNIQUE(user_id, device_id, session_id, request_id)",
        "request_sha256 TEXT NOT NULL",
        "sensitive_result INTEGER NOT NULL",
        "CHECK (sensitive_result = 0 OR result_json IS NULL)",
        "terminal remote operation state is immutable",
        "invalid remote operation state transition",
        "CREATE TABLE IF NOT EXISTS remote_computer_operation_audit",
    ] {
        assert!(
            REMOTE_COMPUTER_OPERATION_JOURNAL_SCHEMA_V28.contains(required),
            "missing operation journal schema invariant: {required}"
        );
    }
    for forbidden in [
        "mobile_token TEXT",
        "client_token TEXT",
        "device_secret TEXT",
        "arguments_json TEXT",
        "password TEXT",
        "otp TEXT",
    ] {
        assert!(
            !REMOTE_COMPUTER_OPERATION_JOURNAL_SCHEMA_V28.contains(forbidden),
            "raw operation secret/input must not be persisted: {forbidden}"
        );
    }

    for required in [
        "remote_computer_operation_create",
        "remote_computer_operation_query",
        "remote_computer_operation_outcome_unknown",
        "remote_computer_operation_cancel",
        "remote_computer_operation_claim",
        "remote_computer_operation_result",
        "remote_operation_controller_authorized",
        "remote_operation_target_authorized",
        "REMOTE_OPERATION_MAX_RESULT_BYTES",
        "sensitive_result_plaintext_forbidden",
        "operation_request_conflict",
    ] {
        assert!(
            REMOTE_COMPUTER_WORKER_SOURCE.contains(required),
            "missing operation journal source invariant: {required}"
        );
    }

    for route in [
        "/v1/computers/:device_id/sessions/:session_id/operations",
        "/v1/computers/:device_id/sessions/:session_id/operations/:request_id/query",
        "/v1/computers/:device_id/sessions/:session_id/operations/:request_id/outcome-unknown",
        "/v1/computers/:device_id/sessions/:session_id/operations/:request_id/cancel",
        "/v1/computers/:device_id/sessions/:session_id/operations/:request_id/claim",
        "/v1/computers/:device_id/sessions/:session_id/operations/:request_id/result",
    ] {
        assert!(
            WORKER_ROUTER_SOURCE.contains(route),
            "operation journal endpoint is not wired: {route}"
        );
    }
}

#[test]
fn remote_computer_exact_tool_grants_are_generation_bound_and_fail_closed() {
    for required in [
        "CREATE TABLE IF NOT EXISTS remote_computer_session_tool_grants",
        "PRIMARY KEY (session_id, tool_name)",
        "generation INTEGER NOT NULL DEFAULT 1",
        "ALTER TABLE remote_computer_operations ADD COLUMN grant_generation INTEGER",
        "remote exact-tool grant binding or generation cannot move backwards",
        "remote_computer_session_close_revoke_tool_grants",
        "CREATE TABLE IF NOT EXISTS remote_computer_tool_grant_audit",
    ] {
        assert!(
            REMOTE_COMPUTER_EXACT_TOOL_GRANTS_SCHEMA_V29.contains(required),
            "missing exact-tool grant schema invariant: {required}"
        );
    }
    for forbidden in [
        "device_secret TEXT",
        "mobile_token TEXT",
        "client_token TEXT",
        "arguments_json TEXT",
        "password TEXT",
        "otp TEXT",
    ] {
        assert!(
            !REMOTE_COMPUTER_EXACT_TOOL_GRANTS_SCHEMA_V29.contains(forbidden),
            "raw credential or tool input must not be persisted: {forbidden}"
        );
    }
    for required in [
        "remote_computer_tool_grant",
        "remote_computer_tool_revoke",
        "remote_operation_tool_grant_generation",
        "tool_grant_forbidden",
        "tool_grant_stale",
        "grant_generation = ?7",
        "g.tool_name = remote_computer_operations.tool_name",
        "g.generation = remote_computer_operations.grant_generation",
        "g.revoked_at IS NULL AND g.expires_at > ?1",
    ] {
        assert!(
            REMOTE_COMPUTER_WORKER_SOURCE.contains(required),
            "missing exact-tool grant source invariant: {required}"
        );
    }
    for route in [
        "/v1/computers/:device_id/sessions/:session_id/tool-grants/grant",
        "/v1/computers/:device_id/sessions/:session_id/tool-grants/revoke",
    ] {
        assert!(
            WORKER_ROUTER_SOURCE.contains(route),
            "exact-tool grant endpoint is not wired: {route}"
        );
    }
}

#[test]
fn remote_computer_service_clients_are_connection_bound_and_secret_safe() {
    for required in [
        "ALTER TABLE remote_computer_clients ADD COLUMN service_connection_id TEXT",
        "ALTER TABLE remote_computer_clients ADD COLUMN service_delegation_id TEXT",
        "remote_computer_clients_service_connection_device_idx",
        "service connection and delegation binding must be stored together",
        "remote service client binding is immutable",
    ] {
        assert!(
            REMOTE_COMPUTER_SERVICE_CLIENT_SCHEMA_V30.contains(required),
            "missing service client schema invariant: {required}"
        );
    }
    for forbidden in [
        "client_token TEXT",
        "mobile_token TEXT",
        "device_secret TEXT",
        "access_token TEXT",
        "refresh_token TEXT",
    ] {
        assert!(
            !REMOTE_COMPUTER_SERVICE_CLIENT_SCHEMA_V30.contains(forbidden),
            "raw credential must not be persisted in service client binding: {forbidden}"
        );
    }

    for required in [
        "authenticated_official_mcp_service",
        "official_mcp_service_claims",
        "service_connection_id",
        "service_delegation_id",
        "remote_controller_client_binding_matches",
        "client_service_binding_mismatch",
    ] {
        assert!(
            REMOTE_COMPUTER_WORKER_SOURCE.contains(required),
            "missing official MCP controller source invariant: {required}"
        );
    }
}

#[test]
fn official_mcp_controller_operations_keep_service_and_target_authority_separate() {
    for required in [
        "pub(super) async fn remote_computer_operation_create",
        "pub(super) async fn remote_computer_operation_query",
        "pub(super) async fn remote_computer_operation_outcome_unknown",
        "pub(super) async fn remote_computer_operation_cancel",
        "remote_controller_account(&request, &context.env).await",
        "remote_controller_client_binding_matches",
        "service_binding.as_ref()",
        "remote_operation_tool_grant_generation",
    ] {
        assert!(
            REMOTE_COMPUTER_WORKER_SOURCE.contains(required),
            "missing official MCP controller operation invariant: {required}"
        );
    }

    let claim_start = REMOTE_COMPUTER_WORKER_SOURCE
        .find("pub(super) async fn remote_computer_operation_claim")
        .expect("operation claim source");
    let result_start = REMOTE_COMPUTER_WORKER_SOURCE
        .find("pub(super) async fn remote_computer_operation_result")
        .expect("operation result source");
    let activate_start = REMOTE_COMPUTER_WORKER_SOURCE
        .find("pub(super) async fn remote_computer_session_activate")
        .expect("session activate source");
    let claim = &REMOTE_COMPUTER_WORKER_SOURCE[claim_start..result_start];
    let result = &REMOTE_COMPUTER_WORKER_SOURCE[result_start..activate_start];
    for (name, section) in [("claim", claim), ("result", result)] {
        assert!(
            section.contains("authenticated_account(&request, &context.env)"),
            "target {name} must remain ordinary-account authenticated"
        );
        assert!(
            section.contains("device_secret"),
            "target {name} must retain device-secret authorization"
        );
        assert!(
            !section.contains("remote_controller_account"),
            "service controller credential must not authorize target {name}"
        );
    }
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
fn official_mcp_service_delegation_is_session_bound_rotating_and_secret_free() {
    for required in [
        "CREATE TABLE IF NOT EXISTS account_service_delegations",
        "CREATE TABLE IF NOT EXISTS account_service_refresh_tokens",
        "parent_session_id TEXT NOT NULL",
        "external_client_id_hash TEXT NOT NULL",
        "current_refresh_token_hash TEXT NOT NULL UNIQUE",
        "UNIQUE(user_id, parent_session_id, service_id, connection_id)",
        "state TEXT NOT NULL CHECK(state IN ('active','used','revoked'))",
    ] {
        assert!(
            ACCOUNT_SERVICE_DELEGATION_SCHEMA_V8.contains(required),
            "missing service delegation invariant: {required}"
        );
    }
    for forbidden in [
        "access_token TEXT",
        "refresh_token TEXT",
        "fabushi_access_token",
        "fabushi_refresh_token",
        "device_secret",
        "mobile_token",
        "client_token",
    ] {
        assert!(
            !ACCOUNT_SERVICE_DELEGATION_SCHEMA_V8
                .to_ascii_lowercase()
                .contains(&forbidden.to_ascii_lowercase()),
            "service delegation state must not persist credential/plain control secret: {forbidden}"
        );
    }
    for required in [
        "authenticated_official_mcp_service",
        "official_mcp_service_token_create",
        "official_mcp_service_token_refresh",
        "official_mcp_service_token_revoke",
        "official_mcp_service_token_introspect",
        "service:official-device-mcp",
        "devices.read",
        "devices.control",
        "new_service_refresh_token",
        "service_refresh_reused",
    ] {
        assert!(
            ACCOUNT_WORKER_SOURCE.contains(required),
            "missing service delegation source invariant: {required}"
        );
    }
    for route in [
        "/v1/auth/service-token",
        "/v1/auth/service-token/refresh",
        "/v1/auth/service-token/revoke",
        "/v1/auth/service-token/introspect",
    ] {
        assert!(
            WORKER_ROUTER_SOURCE.contains(route),
            "service delegation endpoint is not wired: {route}"
        );
    }
    assert!(
        REMOTE_COMPUTER_WORKER_SOURCE.contains(
            "authenticated_official_mcp_service(\n            &request,\n            &context.env,\n            \"devices.read\""
        ),
        "official MCP service credential must be explicitly scoped into device inventory"
    );
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
            "/v1/developer/commerce/miniapps/:mini_app_id/products/batch",
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
            "/v1/developer/commerce/miniapps/:mini_app_id/google/sync",
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
