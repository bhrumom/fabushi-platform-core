ALTER TABLE plugin_releases
ADD COLUMN source_json TEXT NOT NULL DEFAULT '{}';

ALTER TABLE plugin_releases
ADD COLUMN release_manifest_json TEXT NOT NULL DEFAULT '{}';

ALTER TABLE plugin_releases
ADD COLUMN release_manifest_sha256 TEXT NOT NULL DEFAULT '';

ALTER TABLE plugin_releases
ADD COLUMN release_status TEXT NOT NULL DEFAULT 'approved'
CHECK (release_status IN ('staged', 'pending', 'approved', 'rejected', 'revoked', 'deprecated'));

ALTER TABLE plugin_releases
ADD COLUMN revoked_at INTEGER;

ALTER TABLE plugin_releases
ADD COLUMN revocation_reason TEXT;
