# Telegram MTProto bad_server_salt exact wire validation

Status: Stage A implementation contract; executable validation deferred.

## Scope

This spec closes only the fixed-U receive-side wire-validation responsibility for `bad_server_salt#edab447b` inside the existing canonical `telegram-protocol` owner, with existing `telegram-network` receive branches used only as shipping-reachability evidence. It does not claim or modify salt adoption, bad-time recovery, request-history lookup, resend/application semantics, auth-key binding, connection scheduling, gzip handling, or updates difference decoding.

## Fixed-U provenance

- Upstream: `telegramdesktop/tdesktop@f985723b552b30fe1c17d377d57f44e61a15a8c4`.
- `Telegram/SourceFiles/mtproto/scheme/mtproto.tl@76225965bfc794388db241f3f760db8ab79e84f8`
  - `bad_server_salt#edab447b bad_msg_id:long bad_msg_seqno:int error_code:int new_server_salt:long = BadMsgNotification`.
- `Telegram/SourceFiles/mtproto/session_private.cpp@13cbc76ec723386027b5b4dcac79999b791c4614`
  - the `mtpc_bad_server_salt` branch first calls generated `MTPBadMsgNotification::read(from,end)`;
  - only after a successful read does it access `vbad_msg_id`, `vbad_msg_seqno`, `verror_code`, and `vnew_server_salt`, then enter `wasSent`, salt mutation, time update, binding, reconnect, and resend behavior.

## Live checkpoint disposition

At checkpoint `bhrumom/fabushi-platform-core checkpoint/tdrp-telegram-auth-lifecycle-7457eb1c-r16@21ab4e3c4b98a93761c8df525656c09f9b4c404d`, the canonical owner already provides the exact wire parser:

- `native/telegram-protocol/src/api.rs@ebfb3670b5cd097b9ce288782c7954f5cfe9fc43`
  - `BAD_SERVER_SALT_CONSTRUCTOR = 0xedab447b`;
  - typed `BadServerSalt { bad_message_id: i64, bad_message_sequence_number: i32, error_code: i32, new_server_salt: i64 }`;
  - `parse_bad_server_salt` reads those four fields in order and rejects trailing bytes.
- `native/telegram-protocol/src/tl.rs@89256ab35184b6dc06a095ab34abb9f44e95a264`
  - `read_i64` and `read_i32` decode signed little-endian values through fixed-size `take_array`, enforcing exact 8/4/4/8 field widths and fail-closed truncation.
- `native/telegram-protocol/src/lib.rs@abab39e42f100972484191f0e52e9edd9b88c6cf`
  - exports the same parser, type, and constructor.
- `native/telegram-network/src/lib.rs@1fc1b1f69e3e37377400d078f916599e0c7b20bc`
  - both existing receive branches call `parse_bad_server_salt` before their current-request correlation and salt-adoption behavior.

The exact object size is 28 bytes: 4-byte constructor + 8-byte signed `bad_msg_id` + 4-byte signed `bad_msg_seqno` + 4-byte signed `error_code` + 8-byte signed `new_server_salt`.

Therefore this responsibility is **no production delta**. Production parser/network behavior is not changed by this round.

## Requirements

- R1: The existing canonical protocol owner SHALL remain the unique owner of `BAD_SERVER_SALT_CONSTRUCTOR`, typed `BadServerSalt`, and `parse_bad_server_salt(&[u8])`.
- R2: The parser SHALL decode exactly `bad_msg_id:long`, `bad_msg_seqno:int`, `error_code:int`, and `new_server_salt:long` as signed little-endian 8/4/4/8 fields after the 4-byte constructor.
- R3: Wrong constructor, truncation at or within any field, and any trailing bytes SHALL fail closed.
- R4: Exact EOF after the 28th byte SHALL be required for success.
- R5: Existing network receive branches SHALL continue to consume the typed parser before any current-request correlation or salt mutation; those consumers prove shipping reachability but are outside this wire-only ownership claim.
- R6: This wire-only responsibility SHALL NOT claim `wasSent`/historical request lookup, salt adoption/persistence, server-time correction, binding/reconnect, resend, or broader application semantics.
- R7: This wire-only responsibility SHALL NOT introduce a second Telegram runtime, request-history store, session owner, gzip runtime, or difference decoder.
- R8: Desktop provider contract copies SHALL be synchronized byte-for-byte after the canonical authored-only contract is durably committed.

## Failure contract

- Wrong constructor -> typed TL constructor failure.
- Truncation before completing constructor or any 8/4/4/8 field -> typed unexpected-EOF failure.
- Any bytes after the exact four-field object -> trailing-bytes failure.
- Malformed objects SHALL fail before network application code can correlate a request or adopt a salt.
- Valid objects may continue into the already-existing application lifecycle, but this spec makes no claim that salt adoption/retry semantics are complete or verified.

## Verification strategy

Stage A authors source-level external contracts only for:

- exact 28-byte layout;
- signed `i64/i32/i32/i64` extremes;
- exact EOF after `new_server_salt`;
- wrong-constructor rejection;
- constructor and every field truncation boundary;
- trailing-byte rejection.

Do not run build, typecheck, lint, schema validation, generators, unit/contract/integration/E2E tests, packaging, deployment, installation, GitHub Actions, checks, or statuses during Stage A.

## Acceptance criteria

- AC1: Exact fixed-U `bad_server_salt` wire decoding remains owned by the existing `telegram-protocol`.
- AC2: Source inspection confirms exact signed 8/4/4/8 decoding and strict EOF.
- AC3: Existing shipping receive branches consume the same typed parser before application mutation.
- AC4: Salt adoption, bad-time handling, request history, resend, binding and connection behavior remain explicitly separate from the wire-only claim.
- AC5: Canonical and desktop external contracts are byte-identical and remain authored but not run.
- AC6: Production api/tl/lib/network blobs remain unchanged.
- AC7: `validation_status=not-run` and `accepted=false` remain until Stage B executable evidence exists.

No build, typecheck, lint, schema validator, generator, unit/contract/integration/E2E test, package, deployment, installation, GitHub Actions, checks, statuses, or runtime acceptance is executed or queried in Stage A.
