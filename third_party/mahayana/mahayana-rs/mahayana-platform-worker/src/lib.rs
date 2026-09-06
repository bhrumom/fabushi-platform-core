//! Cloudflare Workers entrypoint and testable platform invariants.

#[cfg(any(target_arch = "wasm32", test))]
mod auth;

#[cfg(any(target_arch = "wasm32", test))]
mod capability_access;

#[cfg(any(target_arch = "wasm32", test))]
mod marketplace_route;

pub const PLATFORM_SCHEMA_V1: &str = include_str!("../migrations/0001_platform.sql");
pub const CI_RUNNER_AUTH_SOURCE_V1: &str = include_str!("worker_api/ci_runner.rs");
pub const LISTENER_RELAY_SCHEMA_V5: &str = include_str!("../migrations/0005_listener_relay.sql");
pub const REMOTE_COMPUTER_SCHEMA_V6: &str = include_str!("../migrations/0006_remote_computer.sql");
pub const REMOTE_COMPUTER_CLIENT_TOKEN_SCHEMA_V14: &str =
    include_str!("../migrations/0014_remote_computer_client_tokens.sql");
pub const REMOTE_COMPUTER_INVENTORY_SCHEMA_V15: &str =
    include_str!("../migrations/0015_remote_computer_inventory.sql");
pub const REMOTE_COMPUTER_SESSION_PROVIDER_SCHEMA_V16: &str =
    include_str!("../migrations/0016_remote_computer_session_provider.sql");
pub const REMOTE_COMPUTER_TRANSPORT_CONTRACT_SCHEMA_V17: &str =
    include_str!("../migrations/0017_remote_computer_transport_contract.sql");
pub const MARKETPLACE_ACCOUNT_INSTALL_SCHEMA_V18: &str =
    include_str!("../migrations/0018_marketplace_account_installs.sql");
pub const MARKETPLACE_ROUTE_PROJECTION_SCHEMA_V19: &str =
    include_str!("../migrations/0019_marketplace_route_projection.sql");
pub const WORKSPACE_MESSAGING_SCHEMA_V7: &str =
    include_str!("../migrations/0007_workspace_messaging.sql");
pub const FABUSHI_PAY_SCHEMA_V7: &str = include_str!("../migrations/0007_fabushi_pay.sql");
pub const ACCOUNT_AUTH_SCHEMA_V2: &str =
    include_str!("../account-migrations/0001_account_auth.sql");
pub const ACCOUNT_OAUTH_SCHEMA_V3: &str =
    include_str!("../account-migrations/0002_oauth_identities.sql");
pub const ACCOUNT_OAUTH_STATUS_SCHEMA_V4: &str =
    include_str!("../account-migrations/0003_oauth_attempt_failed_status.sql");
pub const ACCOUNT_REGISTRATION_SCHEMA_V5: &str =
    include_str!("../account-migrations/0004_email_registration_challenges.sql");
pub const ACCOUNT_PRINCIPAL_SCHEMA_V6: &str =
    include_str!("../account-migrations/0005_principals_connections.sql");

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum SchemaError {
    #[error("platform schema is missing required table {0}")]
    MissingTable(&'static str),
    #[error("platform schema must declare Rust-enforced posted journal balance")]
    MissingJournalBalanceInvariant,
    #[error("Fabushi Pay schema must declare Rust-enforced payment journal balance")]
    MissingFabushiPayBalanceInvariant,
    #[error("platform schema must declare Rust-enforced AI usage capacity")]
    MissingUsageCapacityInvariant,
    #[error("platform schema must use integer amounts")]
    FloatingPointAmount,
}

fn require_tables(schema: &str, tables: &[&'static str]) -> Result<(), SchemaError> {
    for table in tables {
        let declaration = format!("CREATE TABLE IF NOT EXISTS {table}");
        if !schema.contains(&declaration) {
            return Err(SchemaError::MissingTable(table));
        }
    }
    Ok(())
}

pub fn validate_platform_schema(schema: &str) -> Result<(), SchemaError> {
    require_tables(
        schema,
        &[
            "wallet_accounts",
            "journal_entries",
            "journal_lines",
            "products",
            "prices",
            "orders",
            "payment_attempts",
            "entitlements",
            "consumption_reservations",
            "refunds",
            "audit_events",
            "ai_usage_budgets",
            "ai_usage_reservations",
            "ai_usage_events",
        ],
    )?;
    if !schema.contains("journal_balance_enforced_by_worker_batch") {
        return Err(SchemaError::MissingJournalBalanceInvariant);
    }
    if !schema.contains("ai_usage_capacity_enforced_by_worker_batch") {
        return Err(SchemaError::MissingUsageCapacityInvariant);
    }
    validate_integer_amounts(schema)
}

pub fn validate_fabushi_pay_schema(schema: &str) -> Result<(), SchemaError> {
    for table in [
        "payment_product_config",
        "payment_intents",
        "payment_webhook_events",
        "fabushi_payment_refunds",
        "payment_disputes",
        "developer_payout_accounts",
        "developer_settlement_releases",
        "developer_payouts",
    ] {
        let declaration = format!("CREATE TABLE IF NOT EXISTS {table}");
        if !schema.contains(&declaration) {
            return Err(SchemaError::MissingTable(table));
        }
    }
    if !schema.contains("fabushi_pay_balance_enforced_by_worker_batch") {
        return Err(SchemaError::MissingFabushiPayBalanceInvariant);
    }
    validate_integer_amounts(schema)
}

fn validate_integer_amounts(schema: &str) -> Result<(), SchemaError> {
    if schema.lines().any(|line| {
        let line = line.trim().to_ascii_lowercase();
        line.contains("amount") && (line.contains(" real") || line.contains(" float"))
    }) {
        return Err(SchemaError::FloatingPointAmount);
    }
    Ok(())
}

pub fn validate_listener_relay_schema(schema: &str) -> Result<(), SchemaError> {
    require_tables(schema, &["listener_registrations", "listener_events"])
}

pub fn validate_remote_computer_schema(schema: &str) -> Result<(), SchemaError> {
    require_tables(
        schema,
        &[
            "remote_computers",
            "remote_computer_clients",
            "remote_computer_sessions",
            "remote_computer_signals",
        ],
    )
}

pub fn validate_marketplace_account_install_schema(schema: &str) -> Result<(), SchemaError> {
    require_tables(
        schema,
        &[
            "marketplace_plugin_projections",
            "account_marketplace_installs",
        ],
    )
}

pub fn validate_workspace_messaging_schema(schema: &str) -> Result<(), SchemaError> {
    require_tables(
        schema,
        &[
            "platform_workspaces",
            "platform_workspace_members",
            "platform_agents",
            "platform_peers",
            "platform_conversations",
            "platform_conversation_members",
            "platform_messages",
            "platform_message_attachments",
            "platform_workspace_audit_events",
        ],
    )
}

pub fn validate_account_auth_schema(schema: &str) -> Result<(), SchemaError> {
    require_tables(
        schema,
        &[
            "account_password_credentials",
            "account_sessions",
            "account_refresh_tokens",
            "account_auth_events",
        ],
    )
}

pub fn validate_account_registration_schema(schema: &str) -> Result<(), SchemaError> {
    require_tables(schema, &["account_email_challenges"])
}

pub fn validate_account_oauth_schema(schema: &str) -> Result<(), SchemaError> {
    require_tables(schema, &["account_identities", "account_oauth_attempts"])
}

pub fn validate_account_principal_schema(schema: &str) -> Result<(), SchemaError> {
    require_tables(
        schema,
        &[
            "account_principals",
            "account_identity_principals",
            "account_contact_points",
            "account_connections",
            "account_connection_grants",
        ],
    )
}

#[cfg(target_arch = "wasm32")]
mod identity_auth;

#[cfg(target_arch = "wasm32")]
mod worker_api;

#[cfg(test)]
#[path = "schema_tests.rs"]
mod tests;
