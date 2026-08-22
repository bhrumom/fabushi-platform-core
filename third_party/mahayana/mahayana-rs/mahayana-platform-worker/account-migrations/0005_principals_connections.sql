-- Mahayana account principal normalization.
--
-- This migration is additive: existing access tokens and account_sessions keep
-- their legacy user_id during the rollout. New platform code resolves that id
-- through account_principals. After every shipped client uses principal-aware
-- tokens, a later migration may stop reading legacy users entirely.

PRAGMA foreign_keys = ON;

CREATE TABLE IF NOT EXISTS account_principals (
    principal_id TEXT PRIMARY KEY,
    legacy_user_id TEXT UNIQUE,
    principal_kind TEXT NOT NULL DEFAULT 'human'
        CHECK (principal_kind IN ('human', 'service')),
    handle TEXT COLLATE NOCASE,
    display_name TEXT,
    avatar_url TEXT,
    status TEXT NOT NULL DEFAULT 'active'
        CHECK (status IN ('active', 'disabled', 'deleted')),
    locale TEXT,
    timezone TEXT,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL,
    deleted_at INTEGER
);

CREATE UNIQUE INDEX IF NOT EXISTS account_principals_handle_idx
    ON account_principals(handle) WHERE handle IS NOT NULL;
CREATE INDEX IF NOT EXISTS account_principals_status_idx
    ON account_principals(status, updated_at);

-- Backfill a stable, namespace-qualified principal id for every historical user.
-- The mapping is deterministic and never exposes the old numeric id as the
-- canonical identifier to new platform tables.
INSERT OR IGNORE INTO account_principals (
    principal_id,
    legacy_user_id,
    handle,
    display_name,
    avatar_url,
    status,
    created_at,
    updated_at
)
SELECT
    'prn_legacy_' || CAST(id AS TEXT),
    CAST(id AS TEXT),
    NULLIF(username, ''),
    COALESCE(NULLIF(nickname, ''), NULLIF(username, '')),
    COALESCE(NULLIF(avatar, ''), NULLIF(alipay_avatar, ''), NULLIF(wechat_headimgurl, '')),
    'active',
    CAST(strftime('%s', COALESCE(created_at, CURRENT_TIMESTAMP)) AS INTEGER),
    CAST(strftime('%s', 'now') AS INTEGER)
FROM users;

-- Login identities remain issuer/subject based. This bridge lets the existing
-- account_identities table migrate independently from the legacy users row.
CREATE TABLE IF NOT EXISTS account_identity_principals (
    identity_id TEXT PRIMARY KEY,
    principal_id TEXT NOT NULL,
    linked_at INTEGER NOT NULL,
    FOREIGN KEY (principal_id) REFERENCES account_principals(principal_id) ON DELETE CASCADE
);

INSERT OR IGNORE INTO account_identity_principals (identity_id, principal_id, linked_at)
SELECT
    i.identity_id,
    p.principal_id,
    CAST(strftime('%s', 'now') AS INTEGER)
FROM account_identities i
JOIN account_principals p ON p.legacy_user_id = i.user_id;

CREATE INDEX IF NOT EXISTS account_identity_principals_principal_idx
    ON account_identity_principals(principal_id);

-- Canonical verified contact points are not login-provider rows and are not
-- columns on the principal. They can be changed/reverified without changing
-- principal identity.
CREATE TABLE IF NOT EXISTS account_contact_points (
    contact_id TEXT PRIMARY KEY,
    principal_id TEXT NOT NULL,
    kind TEXT NOT NULL CHECK (kind IN ('email', 'phone')),
    normalized_value TEXT NOT NULL,
    verified_at INTEGER NOT NULL,
    is_primary INTEGER NOT NULL DEFAULT 0 CHECK (is_primary IN (0, 1)),
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL,
    FOREIGN KEY (principal_id) REFERENCES account_principals(principal_id) ON DELETE CASCADE,
    UNIQUE (kind, normalized_value)
);

CREATE UNIQUE INDEX IF NOT EXISTS account_contact_primary_idx
    ON account_contact_points(principal_id, kind) WHERE is_primary = 1;

-- Seed only data that the legacy account explicitly marked as verified.
INSERT OR IGNORE INTO account_contact_points (
    contact_id,
    principal_id,
    kind,
    normalized_value,
    verified_at,
    is_primary,
    created_at,
    updated_at
)
SELECT
    'contact_email_' || CAST(u.id AS TEXT),
    p.principal_id,
    'email',
    lower(trim(u.email)),
    CAST(strftime('%s', 'now') AS INTEGER),
    1,
    CAST(strftime('%s', 'now') AS INTEGER),
    CAST(strftime('%s', 'now') AS INTEGER)
FROM users u
JOIN account_principals p ON p.legacy_user_id = CAST(u.id AS TEXT)
WHERE u.email_verified = 1 AND u.email IS NOT NULL AND trim(u.email) <> '';

-- Persistent provider authorization is deliberately separate from login
-- identity. OAuth used only to sign in must never create a row here.
CREATE TABLE IF NOT EXISTS account_connections (
    connection_id TEXT PRIMARY KEY,
    principal_id TEXT NOT NULL,
    provider TEXT NOT NULL,
    provider_subject TEXT NOT NULL,
    display_name TEXT,
    scopes_json TEXT NOT NULL DEFAULT '[]',
    credential_ref TEXT NOT NULL,
    status TEXT NOT NULL DEFAULT 'active'
        CHECK (status IN ('active', 'expired', 'revoked', 'error')),
    expires_at INTEGER,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL,
    last_used_at INTEGER,
    revoked_at INTEGER,
    FOREIGN KEY (principal_id) REFERENCES account_principals(principal_id) ON DELETE CASCADE,
    UNIQUE (principal_id, provider, provider_subject)
);

CREATE INDEX IF NOT EXISTS account_connections_principal_idx
    ON account_connections(principal_id, status, provider);

-- Explicit grants model what a connected account may do inside one workspace.
-- The credential lives behind credential_ref; raw provider tokens never belong
-- in this relational table.
CREATE TABLE IF NOT EXISTS account_connection_grants (
    connection_id TEXT NOT NULL,
    workspace_id TEXT NOT NULL,
    capability TEXT NOT NULL,
    granted_at INTEGER NOT NULL,
    revoked_at INTEGER,
    PRIMARY KEY (connection_id, workspace_id, capability),
    FOREIGN KEY (connection_id) REFERENCES account_connections(connection_id) ON DELETE CASCADE
);

CREATE INDEX IF NOT EXISTS account_connection_grants_workspace_idx
    ON account_connection_grants(workspace_id, revoked_at);
