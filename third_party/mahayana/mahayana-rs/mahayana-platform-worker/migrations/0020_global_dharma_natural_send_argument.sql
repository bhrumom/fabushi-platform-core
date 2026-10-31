-- Route natural-language Global Dharma send requests to the MCP tool's canonical
-- `content` argument instead of the generic `input` envelope. The route worker
-- consumes this metadata; explicit slash-command JSON remains unchanged.
UPDATE marketplace_plugin_projections
SET projection_json = json_set(
      projection_json,
      '$.commands[4].naturalLanguageArgument',
      'content'
    ),
    updated_at = CAST(strftime('%s', 'now') AS INTEGER)
WHERE plugin_id = 'global-dharma'
  AND json_valid(projection_json)
  AND json_extract(projection_json, '$.commands[4].name') = 'send';
