-- Mahayana workspace + unified messaging graph.
-- Cloudflare OS-inspired ownership/capability boundaries meet Telegram-style
-- stable peers, conversations, membership state, and ordered messages.
-- Principal ids originate from ACCOUNT_DB and are validated by the Worker;
-- D1 cannot enforce foreign keys across databases.

PRAGMA foreign_keys = ON;

CREATE TABLE IF NOT EXISTS platform_workspaces (
    workspace_id TEXT PRIMARY KEY,
    workspace_kind TEXT NOT NULL DEFAULT 'personal'
        CHECK (workspace_kind IN ('personal', 'team', 'project')),
    owner_principal_id TEXT NOT NULL,
    slug TEXT COLLATE NOCASE,
    title TEXT NOT NULL,
    description TEXT,
    settings_json TEXT NOT NULL DEFAULT '{}',
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL,
    archived_at INTEGER
);

CREATE UNIQUE INDEX IF NOT EXISTS platform_workspaces_personal_owner_idx
    ON platform_workspaces(owner_principal_id) WHERE workspace_kind = 'personal';
CREATE UNIQUE INDEX IF NOT EXISTS platform_workspaces_slug_idx
    ON platform_workspaces(slug) WHERE slug IS NOT NULL;
CREATE INDEX IF NOT EXISTS platform_workspaces_updated_idx
    ON platform_workspaces(updated_at DESC);

CREATE TABLE IF NOT EXISTS platform_workspace_members (
    workspace_id TEXT NOT NULL REFERENCES platform_workspaces(workspace_id) ON DELETE CASCADE,
    principal_id TEXT NOT NULL,
    role TEXT NOT NULL DEFAULT 'member'
        CHECK (role IN ('owner', 'admin', 'member', 'viewer')),
    status TEXT NOT NULL DEFAULT 'active'
        CHECK (status IN ('invited', 'active', 'suspended', 'left')),
    capability_policy_json TEXT NOT NULL DEFAULT '{}',
    joined_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL,
    PRIMARY KEY (workspace_id, principal_id)
);

CREATE INDEX IF NOT EXISTS platform_workspace_members_principal_idx
    ON platform_workspace_members(principal_id, status, updated_at DESC);

CREATE TABLE IF NOT EXISTS platform_agents (
    agent_id TEXT PRIMARY KEY,
    workspace_id TEXT NOT NULL REFERENCES platform_workspaces(workspace_id) ON DELETE CASCADE,
    owner_principal_id TEXT NOT NULL,
    name TEXT NOT NULL,
    description TEXT,
    runtime_kind TEXT NOT NULL DEFAULT 'mahayana',
    model_policy_json TEXT NOT NULL DEFAULT '{}',
    capability_policy_json TEXT NOT NULL DEFAULT '{}',
    status TEXT NOT NULL DEFAULT 'active'
        CHECK (status IN ('active', 'paused', 'archived')),
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL
);

CREATE INDEX IF NOT EXISTS platform_agents_workspace_idx
    ON platform_agents(workspace_id, status, updated_at DESC);

-- A peer is the stable communication identity. Humans and AI agents share one
-- substrate instead of parallel "contacts" and "bots" implementations.
CREATE TABLE IF NOT EXISTS platform_peers (
    peer_id TEXT PRIMARY KEY,
    peer_type TEXT NOT NULL CHECK (peer_type IN ('principal', 'agent', 'system')),
    principal_id TEXT,
    agent_id TEXT REFERENCES platform_agents(agent_id) ON DELETE CASCADE,
    display_name TEXT,
    avatar_url TEXT,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL,
    CHECK (
        (peer_type = 'principal' AND principal_id IS NOT NULL AND agent_id IS NULL) OR
        (peer_type = 'agent' AND agent_id IS NOT NULL AND principal_id IS NULL) OR
        (peer_type = 'system' AND principal_id IS NULL AND agent_id IS NULL)
    )
);

CREATE UNIQUE INDEX IF NOT EXISTS platform_peers_principal_idx
    ON platform_peers(principal_id) WHERE principal_id IS NOT NULL;
CREATE UNIQUE INDEX IF NOT EXISTS platform_peers_agent_idx
    ON platform_peers(agent_id) WHERE agent_id IS NOT NULL;

CREATE TABLE IF NOT EXISTS platform_conversations (
    conversation_id TEXT PRIMARY KEY,
    workspace_id TEXT NOT NULL REFERENCES platform_workspaces(workspace_id) ON DELETE CASCADE,
    conversation_kind TEXT NOT NULL DEFAULT 'direct'
        CHECK (conversation_kind IN ('direct', 'group', 'channel', 'agent')),
    created_by_peer_id TEXT NOT NULL REFERENCES platform_peers(peer_id) ON DELETE RESTRICT,
    title TEXT,
    direct_key TEXT,
    next_message_seq INTEGER NOT NULL DEFAULT 1 CHECK (next_message_seq > 0),
    last_message_seq INTEGER NOT NULL DEFAULT 0 CHECK (last_message_seq >= 0),
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL,
    archived_at INTEGER,
    CHECK (last_message_seq < next_message_seq)
);

CREATE UNIQUE INDEX IF NOT EXISTS platform_conversations_direct_idx
    ON platform_conversations(workspace_id, direct_key)
    WHERE direct_key IS NOT NULL AND conversation_kind IN ('direct', 'agent');
CREATE INDEX IF NOT EXISTS platform_conversations_workspace_idx
    ON platform_conversations(workspace_id, updated_at DESC);

CREATE TABLE IF NOT EXISTS platform_conversation_members (
    conversation_id TEXT NOT NULL REFERENCES platform_conversations(conversation_id) ON DELETE CASCADE,
    peer_id TEXT NOT NULL REFERENCES platform_peers(peer_id) ON DELETE CASCADE,
    role TEXT NOT NULL DEFAULT 'member'
        CHECK (role IN ('owner', 'admin', 'member', 'readonly')),
    joined_at INTEGER NOT NULL,
    left_at INTEGER,
    last_read_seq INTEGER NOT NULL DEFAULT 0 CHECK (last_read_seq >= 0),
    notification_policy_json TEXT NOT NULL DEFAULT '{}',
    PRIMARY KEY (conversation_id, peer_id)
);

CREATE INDEX IF NOT EXISTS platform_conversation_members_peer_idx
    ON platform_conversation_members(peer_id, left_at);

CREATE TABLE IF NOT EXISTS platform_messages (
    message_id TEXT PRIMARY KEY,
    conversation_id TEXT NOT NULL REFERENCES platform_conversations(conversation_id) ON DELETE CASCADE,
    seq INTEGER NOT NULL CHECK (seq > 0),
    sender_peer_id TEXT NOT NULL REFERENCES platform_peers(peer_id) ON DELETE RESTRICT,
    reply_to_message_id TEXT REFERENCES platform_messages(message_id) ON DELETE SET NULL,
    message_kind TEXT NOT NULL DEFAULT 'text'
        CHECK (message_kind IN ('text', 'markdown', 'tool', 'event', 'file', 'system')),
    content_text TEXT,
    content_json TEXT,
    client_nonce TEXT,
    created_at INTEGER NOT NULL,
    edited_at INTEGER,
    deleted_at INTEGER,
    UNIQUE (conversation_id, seq),
    UNIQUE (conversation_id, client_nonce),
    CHECK (content_text IS NOT NULL OR content_json IS NOT NULL)
);

CREATE INDEX IF NOT EXISTS platform_messages_conversation_idx
    ON platform_messages(conversation_id, seq DESC);
CREATE INDEX IF NOT EXISTS platform_messages_sender_idx
    ON platform_messages(sender_peer_id, created_at DESC);

CREATE TABLE IF NOT EXISTS platform_message_attachments (
    attachment_id TEXT PRIMARY KEY,
    message_id TEXT NOT NULL REFERENCES platform_messages(message_id) ON DELETE CASCADE,
    object_key TEXT NOT NULL,
    media_type TEXT,
    file_name TEXT,
    byte_size INTEGER CHECK (byte_size IS NULL OR byte_size >= 0),
    sha256 TEXT,
    metadata_json TEXT NOT NULL DEFAULT '{}',
    created_at INTEGER NOT NULL
);

CREATE INDEX IF NOT EXISTS platform_message_attachments_message_idx
    ON platform_message_attachments(message_id);

-- Workspace-level audit is distinct from commerce ledger audit. It records
-- authorization-relevant changes to membership, agents, conversations and
-- external capability grants without putting them in message history.
CREATE TABLE IF NOT EXISTS platform_workspace_audit_events (
    event_id TEXT PRIMARY KEY,
    workspace_id TEXT REFERENCES platform_workspaces(workspace_id) ON DELETE SET NULL,
    actor_principal_id TEXT,
    actor_peer_id TEXT REFERENCES platform_peers(peer_id) ON DELETE SET NULL,
    action TEXT NOT NULL,
    subject_type TEXT,
    subject_id TEXT,
    payload_json TEXT NOT NULL DEFAULT '{}',
    created_at INTEGER NOT NULL
);

CREATE INDEX IF NOT EXISTS platform_workspace_audit_idx
    ON platform_workspace_audit_events(workspace_id, created_at DESC);
