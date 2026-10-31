-- Account-authoritative Mini App installation projection.
-- Account identities live in ACCOUNT_DB, so account_user_id intentionally has no
-- cross-D1 foreign key. Plugin identity remains constrained by PLATFORM_DB.
CREATE TABLE IF NOT EXISTS marketplace_plugin_projections (
    plugin_id TEXT PRIMARY KEY REFERENCES marketplace_plugins(plugin_id) ON DELETE CASCADE,
    projection_json TEXT NOT NULL,
    updated_at INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS account_marketplace_installs (
    account_user_id TEXT NOT NULL,
    plugin_id TEXT NOT NULL REFERENCES marketplace_plugins(plugin_id) ON DELETE RESTRICT,
    platform TEXT NOT NULL,
    installed_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL,
    PRIMARY KEY (account_user_id, plugin_id)
);

CREATE INDEX IF NOT EXISTS account_marketplace_installs_account_idx
ON account_marketplace_installs(account_user_id, updated_at DESC);

INSERT INTO marketplace_plugin_projections (plugin_id, projection_json, updated_at)
VALUES (
    'global-dharma',
    '{"protocol":"fabushi.miniapp.manifest.v2","id":"global-dharma","version":"1.0.0","title":"全球法布施","description":"同时提供图形界面、自然语言、MCP 与 CLI 的全平台法布施小程序。","bot":{"id":"global-dharma-bot","username":"global_dharma_bot","displayName":"全球法布施","description":"用自然语言或 / 命令驱动全球发送、本地转经轮与场能模式。","conversationId":"miniapp:global-dharma","managedBy":"bot-father","mainApp":true,"naturalLanguage":true,"menuButton":{"text":"打开小程序","action":"open-miniapp","miniAppId":"global-dharma"}}}',
    1788736800
)
ON CONFLICT(plugin_id) DO UPDATE SET
    projection_json = excluded.projection_json,
    updated_at = excluded.updated_at;
