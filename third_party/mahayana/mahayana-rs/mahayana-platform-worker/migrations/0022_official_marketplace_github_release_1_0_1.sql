-- Move the official Mini App catalog to the GitHub immutable install contract.
-- Package bytes stay outside D1; this migration records only the GitHub source,
-- release asset, digest, size, and the manifest needed for verified installs.

UPDATE marketplace_plugins
SET latest_version = '1.0.0',
    updated_at = 1789317964,
    platforms_json = '["desktop","mobile","web","cli","chrome-extension"]'
WHERE plugin_id = 'global-dharma';

UPDATE marketplace_plugins
SET latest_version = '1.0.1',
    updated_at = 1789317964,
    platforms_json = '["desktop","mobile","web","cli","chrome-extension"]'
WHERE plugin_id = 'faliu-flashcards';

UPDATE marketplace_plugins
SET latest_version = '1.0.0',
    updated_at = 1789317964,
    platforms_json = '["desktop","mobile","web","cli","chrome-extension"]'
WHERE plugin_id = 'platform-publish';

UPDATE marketplace_plugins
SET latest_version = '1.0.1',
    updated_at = 1789317964,
    platforms_json = '["desktop","web","cli","chrome-extension"]'
WHERE plugin_id = 'hermes-installer';

UPDATE marketplace_plugins
SET latest_version = '1.0.0',
    updated_at = 1789317964,
    platforms_json = '["desktop","mobile","web","cli","chrome-extension"]'
WHERE plugin_id = 'bot-father';

UPDATE marketplace_plugins
SET latest_version = '1.0.0',
    updated_at = 1789317964,
    platforms_json = '["desktop","mobile","web","cli","chrome-extension"]'
WHERE plugin_id = 'mahayana-assistant';

UPDATE marketplace_plugins
SET latest_version = '1.0.1',
    updated_at = 1789317964,
    platforms_json = '["desktop","cli","chrome-extension"]'
WHERE plugin_id = 'chatgpt-auto-confirm';

UPDATE marketplace_plugins
SET latest_version = '1.0.0',
    updated_at = 1789317964,
    platforms_json = '["desktop","cli"]'
WHERE plugin_id = 'douyin-batch-downloader';

-- Do not leave the three checksum-invalid legacy archives installable by an
-- explicit version request. Their replacement 1.0.1 releases below are the
-- only approved versions for these plugins after this migration.
UPDATE plugin_releases
SET release_status = 'revoked',
    revoked_at = 1789317964,
    revocation_reason = 'legacy archive failed gzip integrity validation; replaced by immutable GitHub release'
WHERE (plugin_id = 'faliu-flashcards' AND version = '1.0.0')
   OR (plugin_id = 'hermes-installer' AND version = '1.0.0')
   OR (plugin_id = 'chatgpt-auto-confirm' AND version = '1.0.0+codex.20260810093000');

-- The five existing valid archives remain 1.0.0 releases, but their catalog
-- pointers now resolve to the accepted immutable commit and GitHub source.
UPDATE plugin_releases
SET package_key = 'https://raw.githubusercontent.com/bhrumom/fabushi/cc23420c56c98f7857b731832281c212203ce60c/marketplace/packages/global-dharma/1.0.0/app.tar.gz',
    package_sha256 = '43de877dc87b5dff306164eb143baad545ef40bea2247f28cbe21616829478be',
    package_size = 1827,
    tuf_target_path = 'official/global-dharma/1.0.0',
    published_at = 1789317964,
    deployment_url = 'https://raw.githubusercontent.com/bhrumom/fabushi/cc23420c56c98f7857b731832281c212203ce60c/marketplace/packages/global-dharma/1.0.0/app.tar.gz',
    source_json = '{"marketplaceHostsPackage":false,"provider":"github","repository":"https://github.com/bhrumom/fabushi","sourceRef":"cc23420c56c98f7857b731832281c212203ce60c"}',
    release_manifest_json = '{"artifacts":[{"entry":"index.html","format":"tar-gz","id":"global-dharma-universal-ui","platforms":["desktop","mobile","web","cli"],"runtime":"local-web","sha256":"43de877dc87b5dff306164eb143baad545ef40bea2247f28cbe21616829478be","size":1827,"source":{"type":"https","url":"https://raw.githubusercontent.com/bhrumom/fabushi/cc23420c56c98f7857b731832281c212203ce60c/marketplace/packages/global-dharma/1.0.0/app.tar.gz"}}],"entry":"index.html","installMode":"package","permissions":["network","local-execution"],"pluginId":"global-dharma","protocol":"mahayana.external-release.v1","publisher":{"displayName":"Fabushi 官方","id":"fabushi-official","verified":true},"releaseStatus":"approved","runtime":"local-web","schemaVersion":1,"source":{"marketplaceHostsPackage":false,"provider":"github","repository":"https://github.com/bhrumom/fabushi","sourceRef":"cc23420c56c98f7857b731832281c212203ce60c"},"version":"1.0.0"}',
    release_manifest_sha256 = 'c1b90f975790f6355b36476c81e388442f0852f3a2394857cae4507df180588e',
    release_status = 'approved'
WHERE plugin_id = 'global-dharma' AND version = '1.0.0';

UPDATE plugin_releases
SET package_key = 'https://raw.githubusercontent.com/bhrumom/fabushi/cc23420c56c98f7857b731832281c212203ce60c/marketplace/packages/platform-publish/1.0.0/app.tar.gz',
    package_sha256 = '4ded6de4cada43998f5fae2f226c4bea50b3fbc62a609f13c87a2102efb10802',
    package_size = 1742,
    tuf_target_path = 'official/platform-publish/1.0.0',
    published_at = 1789317964,
    deployment_url = 'https://raw.githubusercontent.com/bhrumom/fabushi/cc23420c56c98f7857b731832281c212203ce60c/marketplace/packages/platform-publish/1.0.0/app.tar.gz',
    source_json = '{"marketplaceHostsPackage":false,"provider":"github","repository":"https://github.com/bhrumom/fabushi","sourceRef":"cc23420c56c98f7857b731832281c212203ce60c"}',
    release_manifest_json = '{"artifacts":[{"entry":"index.html","format":"tar-gz","id":"platform-publish-universal-ui","platforms":["desktop","mobile","web","cli"],"runtime":"local-web","sha256":"4ded6de4cada43998f5fae2f226c4bea50b3fbc62a609f13c87a2102efb10802","size":1742,"source":{"type":"https","url":"https://raw.githubusercontent.com/bhrumom/fabushi/cc23420c56c98f7857b731832281c212203ce60c/marketplace/packages/platform-publish/1.0.0/app.tar.gz"}}],"entry":"index.html","installMode":"package","permissions":["network","publish-content"],"pluginId":"platform-publish","protocol":"mahayana.external-release.v1","publisher":{"displayName":"Fabushi 官方","id":"fabushi-official","verified":true},"releaseStatus":"approved","runtime":"local-web","schemaVersion":1,"source":{"marketplaceHostsPackage":false,"provider":"github","repository":"https://github.com/bhrumom/fabushi","sourceRef":"cc23420c56c98f7857b731832281c212203ce60c"},"version":"1.0.0"}',
    release_manifest_sha256 = 'ddd6ea5a5ffd722949db92e5c477f74f5b5b5654d132a98d8b52e4dbcef5cd15',
    release_status = 'approved'
WHERE plugin_id = 'platform-publish' AND version = '1.0.0';

UPDATE plugin_releases
SET package_key = 'https://raw.githubusercontent.com/bhrumom/fabushi/cc23420c56c98f7857b731832281c212203ce60c/marketplace/packages/bot-father/1.0.0/app.tar.gz',
    package_sha256 = '8439c9c7ffe03791177bb5b9cbfd425ffb794b741d9e17c9cc2cadc09fbb7880',
    package_size = 1805,
    tuf_target_path = 'official/bot-father/1.0.0',
    published_at = 1789317964,
    deployment_url = 'https://raw.githubusercontent.com/bhrumom/fabushi/cc23420c56c98f7857b731832281c212203ce60c/marketplace/packages/bot-father/1.0.0/app.tar.gz',
    source_json = '{"marketplaceHostsPackage":false,"provider":"github","repository":"https://github.com/bhrumom/fabushi","sourceRef":"cc23420c56c98f7857b731832281c212203ce60c"}',
    release_manifest_json = '{"artifacts":[{"entry":"index.html","format":"tar-gz","id":"bot-father-universal-ui","platforms":["desktop","mobile","web","cli"],"runtime":"local-web","sha256":"8439c9c7ffe03791177bb5b9cbfd425ffb794b741d9e17c9cc2cadc09fbb7880","size":1805,"source":{"type":"https","url":"https://raw.githubusercontent.com/bhrumom/fabushi/cc23420c56c98f7857b731832281c212203ce60c/marketplace/packages/bot-father/1.0.0/app.tar.gz"}}],"entry":"index.html","installMode":"package","permissions":["workspace-write","network","github"],"pluginId":"bot-father","protocol":"mahayana.external-release.v1","publisher":{"displayName":"Fabushi 官方","id":"fabushi-official","verified":true},"releaseStatus":"approved","runtime":"local-web","schemaVersion":1,"source":{"marketplaceHostsPackage":false,"provider":"github","repository":"https://github.com/bhrumom/fabushi","sourceRef":"cc23420c56c98f7857b731832281c212203ce60c"},"version":"1.0.0"}',
    release_manifest_sha256 = '709aff800af1a63d13cadb040a470182b4b802b0ff6b62a32a75648d66e434a1',
    release_status = 'approved'
WHERE plugin_id = 'bot-father' AND version = '1.0.0';

UPDATE plugin_releases
SET package_key = 'https://raw.githubusercontent.com/bhrumom/fabushi/cc23420c56c98f7857b731832281c212203ce60c/marketplace/packages/mahayana-assistant/1.0.0/app.tar.gz',
    package_sha256 = 'e175196bd10827d7e22cec1aa56bcb15540b03ce17c8cb84a7beac8719434d7b',
    package_size = 1777,
    tuf_target_path = 'official/mahayana-assistant/1.0.0',
    published_at = 1789317964,
    deployment_url = 'https://raw.githubusercontent.com/bhrumom/fabushi/cc23420c56c98f7857b731832281c212203ce60c/marketplace/packages/mahayana-assistant/1.0.0/app.tar.gz',
    source_json = '{"marketplaceHostsPackage":false,"provider":"github","repository":"https://github.com/bhrumom/fabushi","sourceRef":"cc23420c56c98f7857b731832281c212203ce60c"}',
    release_manifest_json = '{"artifacts":[{"entry":"index.html","format":"tar-gz","id":"mahayana-assistant-universal-ui","platforms":["desktop","mobile","web","cli"],"runtime":"local-web","sha256":"e175196bd10827d7e22cec1aa56bcb15540b03ce17c8cb84a7beac8719434d7b","size":1777,"source":{"type":"https","url":"https://raw.githubusercontent.com/bhrumom/fabushi/cc23420c56c98f7857b731832281c212203ce60c/marketplace/packages/mahayana-assistant/1.0.0/app.tar.gz"}}],"entry":"index.html","installMode":"package","permissions":["marketplace-read"],"pluginId":"mahayana-assistant","protocol":"mahayana.external-release.v1","publisher":{"displayName":"Fabushi 官方","id":"fabushi-official","verified":true},"releaseStatus":"approved","runtime":"local-web","schemaVersion":1,"source":{"marketplaceHostsPackage":false,"provider":"github","repository":"https://github.com/bhrumom/fabushi","sourceRef":"cc23420c56c98f7857b731832281c212203ce60c"},"version":"1.0.0"}',
    release_manifest_sha256 = '1411f5c25fc6f9c9c8eb436fe1468327d1d4c542edf5205ae57bf71f92f0c62a',
    release_status = 'approved'
WHERE plugin_id = 'mahayana-assistant' AND version = '1.0.0';

UPDATE plugin_releases
SET package_key = 'https://raw.githubusercontent.com/bhrumom/fabushi/cc23420c56c98f7857b731832281c212203ce60c/marketplace/packages/douyin-batch-downloader/1.0.0/app.tar.gz',
    package_sha256 = '6784eb6ade91ef75ff61717a232dd154c7a3fb28c093ce330bc7ca4857ace473',
    package_size = 3069,
    tuf_target_path = 'official/douyin-batch-downloader/1.0.0',
    published_at = 1789317964,
    deployment_url = 'https://raw.githubusercontent.com/bhrumom/fabushi/cc23420c56c98f7857b731832281c212203ce60c/marketplace/packages/douyin-batch-downloader/1.0.0/app.tar.gz',
    source_json = '{"marketplaceHostsPackage":false,"provider":"github","repository":"https://github.com/bhrumom/fabushi","sourceRef":"cc23420c56c98f7857b731832281c212203ce60c"}',
    release_manifest_json = '{"artifacts":[{"entry":"index.html","format":"tar-gz","id":"douyin-batch-downloader-universal-ui","platforms":["desktop","cli"],"runtime":"local-web","sha256":"6784eb6ade91ef75ff61717a232dd154c7a3fb28c093ce330bc7ca4857ace473","size":3069,"source":{"type":"https","url":"https://raw.githubusercontent.com/bhrumom/fabushi/cc23420c56c98f7857b731832281c212203ce60c/marketplace/packages/douyin-batch-downloader/1.0.0/app.tar.gz"}}],"entry":"index.html","installMode":"package","permissions":["network","local-files","local-execution"],"pluginId":"douyin-batch-downloader","protocol":"mahayana.external-release.v1","publisher":{"displayName":"Fabushi 官方","id":"fabushi-official","verified":true},"releaseStatus":"approved","runtime":"local-web","schemaVersion":1,"source":{"marketplaceHostsPackage":false,"provider":"github","repository":"https://github.com/bhrumom/fabushi","sourceRef":"cc23420c56c98f7857b731832281c212203ce60c"},"version":"1.0.0"}',
    release_manifest_sha256 = 'd14be251163c305b76940a0696168893a1f78e30f6efe1254ab9ede5d1aea541',
    release_status = 'approved'
WHERE plugin_id = 'douyin-batch-downloader' AND version = '1.0.0';

-- These releases were rebuilt from the legacy archives and published as
-- immutable GitHub assets in marketplace-v1.0.1-cc23420c56c9.
INSERT OR IGNORE INTO plugin_releases
  (plugin_id, version, package_key, package_sha256, package_size, tuf_target_path, published_at, deployment_url,
   source_json, release_manifest_json, release_manifest_sha256, release_status)
VALUES
('faliu-flashcards','1.0.1','https://github.com/bhrumom/fabushi/releases/download/marketplace-v1.0.1-cc23420c56c9/faliu-flashcards-1.0.1.tar.gz','fb2a8fa187fde312069c9facb49657c366cfa4176f27a90abff5aa407e260356',1729,'official/faliu-flashcards/1.0.1',1789317964,'https://github.com/bhrumom/fabushi/releases/download/marketplace-v1.0.1-cc23420c56c9/faliu-flashcards-1.0.1.tar.gz','{"marketplaceHostsPackage":false,"provider":"github","repository":"https://github.com/bhrumom/fabushi","sourceRef":"cc23420c56c98f7857b731832281c212203ce60c"}','{"artifacts":[{"entry":"index.html","format":"tar-gz","id":"faliu-flashcards-universal-ui","platforms":["desktop","mobile","web","cli","chrome-extension"],"runtime":"local-web","sha256":"fb2a8fa187fde312069c9facb49657c366cfa4176f27a90abff5aa407e260356","size":1729,"source":{"asset":"faliu-flashcards-1.0.1.tar.gz","repository":"bhrumom/fabushi","tag":"marketplace-v1.0.1-cc23420c56c9","type":"github-release"}}],"entry":"index.html","installMode":"package","permissions":["account-storage"],"pluginId":"faliu-flashcards","protocol":"mahayana.external-release.v1","publisher":{"displayName":"Fabushi 官方","id":"fabushi-official","verified":true},"releaseStatus":"approved","runtime":"local-web","schemaVersion":1,"source":{"marketplaceHostsPackage":false,"provider":"github","repository":"https://github.com/bhrumom/fabushi","sourceRef":"cc23420c56c98f7857b731832281c212203ce60c"},"version":"1.0.1"}','72c77e5be617d63ebb9b4e463d328a05f42f574d2ef65d998ea8885ba5937b5f','approved'),
('hermes-installer','1.0.1','https://github.com/bhrumom/fabushi/releases/download/marketplace-v1.0.1-cc23420c56c9/hermes-installer-1.0.1.tar.gz','e693cb2378d580cb86d88fb391a04b8c96dcf6614b445c32339bfb7358e0c4cd',1731,'official/hermes-installer/1.0.1',1789317964,'https://github.com/bhrumom/fabushi/releases/download/marketplace-v1.0.1-cc23420c56c9/hermes-installer-1.0.1.tar.gz','{"marketplaceHostsPackage":false,"provider":"github","repository":"https://github.com/bhrumom/fabushi","sourceRef":"cc23420c56c98f7857b731832281c212203ce60c"}','{"artifacts":[{"entry":"index.html","format":"tar-gz","id":"hermes-installer-universal-ui","platforms":["desktop","web","cli","chrome-extension"],"runtime":"local-web","sha256":"e693cb2378d580cb86d88fb391a04b8c96dcf6614b445c32339bfb7358e0c4cd","size":1731,"source":{"asset":"hermes-installer-1.0.1.tar.gz","repository":"bhrumom/fabushi","tag":"marketplace-v1.0.1-cc23420c56c9","type":"github-release"}}],"entry":"index.html","installMode":"package","permissions":["local-execution","secret-store","network"],"pluginId":"hermes-installer","protocol":"mahayana.external-release.v1","publisher":{"displayName":"Fabushi 官方","id":"fabushi-official","verified":true},"releaseStatus":"approved","runtime":"local-web","schemaVersion":1,"source":{"marketplaceHostsPackage":false,"provider":"github","repository":"https://github.com/bhrumom/fabushi","sourceRef":"cc23420c56c98f7857b731832281c212203ce60c"},"version":"1.0.1"}','f31fa77749c84e35e49dad7d8a6e6cbf6eac39e6890b2e1a4203e1f3911682eb','approved'),
('chatgpt-auto-confirm','1.0.1','https://github.com/bhrumom/fabushi/releases/download/marketplace-v1.0.1-cc23420c56c9/chatgpt-auto-confirm-1.0.1.tar.gz','ce5beae5f3b8a29dccb65cb91744f2a82bb19186c3f7031ca75f405ab4effb76',983,'official/chatgpt-auto-confirm/1.0.1',1789317964,'https://github.com/bhrumom/fabushi/releases/download/marketplace-v1.0.1-cc23420c56c9/chatgpt-auto-confirm-1.0.1.tar.gz','{"marketplaceHostsPackage":false,"provider":"github","repository":"https://github.com/bhrumom/fabushi","sourceRef":"cc23420c56c98f7857b731832281c212203ce60c"}','{"artifacts":[{"entry":"index.html","format":"tar-gz","id":"chatgpt-auto-confirm-universal-ui","platforms":["desktop","cli","chrome-extension"],"runtime":"local-web","sha256":"ce5beae5f3b8a29dccb65cb91744f2a82bb19186c3f7031ca75f405ab4effb76","size":983,"source":{"asset":"chatgpt-auto-confirm-1.0.1.tar.gz","repository":"bhrumom/fabushi","tag":"marketplace-v1.0.1-cc23420c56c9","type":"github-release"}}],"entry":"index.html","installMode":"package","permissions":["local-execution","accessibility","browser-control"],"pluginId":"chatgpt-auto-confirm","protocol":"mahayana.external-release.v1","publisher":{"displayName":"Fabushi 官方","id":"fabushi-official","verified":true},"releaseStatus":"approved","runtime":"local-web","schemaVersion":1,"source":{"marketplaceHostsPackage":false,"provider":"github","repository":"https://github.com/bhrumom/fabushi","sourceRef":"cc23420c56c98f7857b731832281c212203ce60c"},"version":"1.0.1"}','3705f8e0195e76098d1e4efac28d902ffb4b182e139323db9b966cba08a3595f','approved');
