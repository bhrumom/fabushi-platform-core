-- Preserve the marketplace publisher identity in marketplaceSource.provider.
-- GitHub is the immutable transport/source and is already represented by the
-- repository, sourceRef, release manifest, and github-immutable install fields.
-- This additive repair also fixes databases where migration 0022 was already
-- applied before the compatibility contract was clarified.
UPDATE plugin_releases
SET source_json = '{"marketplaceHostsPackage":false,"provider":"fabushi-official","repository":"https://github.com/bhrumom/fabushi","commit":"cc23420c56c98f7857b731832281c212203ce60c","sourceRef":"cc23420c56c98f7857b731832281c212203ce60c"}'
WHERE source_json LIKE '%cc23420c56c98f7857b731832281c212203ce60c%'
  AND plugin_id IN (
    'global-dharma',
    'platform-publish',
    'bot-father',
    'mahayana-assistant',
    'douyin-batch-downloader',
    'faliu-flashcards',
    'hermes-installer',
    'chatgpt-auto-confirm'
  );
