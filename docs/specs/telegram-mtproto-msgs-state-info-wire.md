# Telegram MTProto msgs_state_info wire parser contract

Status: active  
Owner: shared telegram-protocol response/TL codec  
Last updated: 2026-11-01  
Related issue/task/PR: Fabushi migration task 7457eb1c-11a3-4626-8563-a3e495872536 round 53

## 1. Context / problem

Fixed upstream U defines `msgs_state_info#04deb57d req_msg_id:long info:string = MsgsStateInfo`. In the shipping
Telegram Desktop `SessionPrivate::handle` service-message path, `mtpc_msgs_state_info` is first decoded through
`MTPMsgsStateInfo::read(from, end)`; only after successful typed decoding are `req_msg_id` and `info` handed to
`_stateAndResendRequests` request-history logic and `handleMsgsStates`.

Fabushi already has a source-neutral typed parser in the existing shared `telegram-protocol` owner:
`parse_msgs_state_info` checks constructor `0x04deb57d`, reads `req_msg_id:long` as i64, reads the TL
`info:string` wire payload as opaque bytes, and rejects trailing bytes. The fixed-U shipping consumer treats
`info` as binary message-state bytes rather than human text, so this boundary must not impose UTF-8 validation.

No production behavior delta is required for this isolated wire-parser responsibility. The current external protocol
contract does not independently lock this parser's constructor, signed-long, binary TL string, long-form length,
truncation, padding or trailing-byte behavior.

## 2. Goal

Close the fixed-U `msgs_state_info#04deb57d` exact typed parser / TL string framing / trailing-validation
responsibility in the existing `telegram-protocol` owner, strictly separated from arbitrary request-history and
application semantics.

## 3. Non-goals / out of scope

- No `_stateAndResendRequests`, `wasSent`, `haveSent`, `_resendingIds`, `toSend`, `_sentContainers` or
  other arbitrary multi-request history ownership.
- No replay/resend policy, request correlation, callback dispatch, retry, cancellation, connection scheduling/racing,
  timeout/backoff or send lifecycle changes.
- No temporary/permanent auth-key binding, expiry, renewal, persistence, destruction or teardown.
- No `gzip_packed` decompression.
- No complete updates difference/differenceSlice decoder or application.
- No change to `handleMsgsStates` application semantics.
- No Stage-A build, typecheck, lint, schema validation, generator execution, unit/contract/integration/E2E execution,
  Actions/status/check query, packaging, deployment, installation or runtime acceptance.

## 4. Requirements

- R1: Preserve `MSGS_STATE_INFO_CONSTRUCTOR = 0x04deb57d`.
- R2: `parse_msgs_state_info(input)` SHALL require that exact constructor and reject another constructor through the
  existing `UnexpectedConstructor` path.
- R3: The parser SHALL read exactly one signed little-endian `req_msg_id:long` into `request_message_id: i64`.
- R4: The parser SHALL decode `info:string` with the existing TL bytes/string length-prefix and padding rules and
  preserve the payload byte-for-byte as `states: Vec<u8>`, including empty and non-UTF-8 values. It SHALL NOT
  reinterpret this fixed-U state payload as Unicode text.
- R5: Constructor, long, TL length header, TL payload and required padding truncation SHALL fail closed through the
  existing `TlError` path. Existing `TlReader` configured byte-length limits remain authoritative.
- R6: After the TL string payload and padding, any remaining bytes SHALL fail closed through the existing
  `ApiRequestError::InvalidMessageState` path.
- R7: Exact valid input SHALL terminate at EOF and parsing SHALL be side-effect free.
- R8: The desktop tracked provider SHALL remain a mirror of the canonical parser/facade/contracts; no second parser
  or service-message lifecycle owner is introduced.
- R9: Canonical and desktop tracked contracts SHALL add authored-only positive and negative/boundary cases covering
  exact fields, binary short-form TL string, long-form TL string, wrong constructor, truncation, padding truncation,
  trailing bytes and exact EOF. They SHALL NOT be executed in Stage A.
- R10: This isolated wire slice SHALL NOT be used to claim request-history/application, connection scheduling,
  auth-key lifecycle, gzip or complete difference parity.

## 5. Current state

At platform-core checkpoint `e448cb7307ded83d9c6eab34fff0ef31f48e0e79`:

- `native/telegram-protocol/src/api.rs@ebfb3670b5cd097b9ce288782c7954f5cfe9fc43` defines
  `MSGS_STATE_INFO_CONSTRUCTOR = 0x04deb57d`, `MsgsStateInfo { request_message_id, states }`, and
  `parse_msgs_state_info`.
- The parser uses `TlReader` to require the constructor, read i64, decode `info` with `read_bytes()`, and reject
  `!reader.is_finished()` as `InvalidMessageState`.
- `native/telegram-protocol/src/tl.rs@89256ab35184b6dc06a095ab34abb9f44e95a264` owns TL bytes/string
  short/long length framing, byte-length limits, body/padding consumption and EOF errors.
- `native/telegram-protocol/src/lib.rs@abab39e42f100972484191f0e52e9edd9b88c6cf` exports the same
  parser/type/constructor.
- `native/telegram-protocol/tests/protocol_contract.rs@d171a3ab9a43955a693c87162f80c2667fc397ce` has no
  `msgs_state_info` parser contract.
- Desktop tracked production api/lib and contract blobs are byte-identical to the canonical blobs above.
- The existing parity ledger has no `MSGS-STATE-INFO` responsibility row. Current coverage is
  `responsibilities_total=93 / unknown=16172 / unread=16145 / omitted=0`.

## 6. Target state

Production `api.rs`, `tl.rs`, `lib.rs`, networking and runtime behavior remain unchanged. A durable spec and
authored-only external contracts independently lock exact typed `MsgsStateInfo` parsing and fail-closed malformed /
trailing behavior. Desktop mirrors the canonical authored contract file. Existing parity/P0/source-closure/evidence
surfaces record one separate no-production-delta responsibility as
`implemented / validation_status=not-run / accepted=false`.

## 7. Architecture and ownership boundaries

- `native/telegram-protocol/src/api.rs`: existing canonical `MsgsStateInfo` typed parser.
- `native/telegram-protocol/src/tl.rs`: existing TL bytes/string length/padding/EOF primitive owner.
- `native/telegram-protocol/src/lib.rs`: existing source-neutral public facade.
- `native/telegram-protocol/tests/protocol_contract.rs`: canonical authored-only Stage-A wire contracts.
- Desktop tracked telegram-protocol source/contracts: existing consumer mirror only.
- Telegram Desktop request-history/application, connection lifecycle, auth-key lifecycle, gzip and updates
  difference logic: source behavior reference only; not recreated by this parser slice.

No new owner or ADR is required.

## 8. Interfaces / contracts / schemas / data flow

`input: &[u8]`
→ exact constructor `0x04DEB57D`
→ `req_msg_id:i64`
→ one TL string/bytes field with canonical short/long framing and padding consumption
→ preserve raw `info` bytes as `states`
→ exact EOF
→ `Ok(MsgsStateInfo { request_message_id, states })`.

Fixed-U shipping then uses `request_message_id` to query its request-history map and interprets `states` against
the original state/resend request. Those history/application steps are outside this parser.

## 9. Constraints and non-functional requirements

- Upstream U remains `telegramdesktop/tdesktop@f985723b552b30fe1c17d377d57f44e61a15a8c4`.
- Schema blob remains `Telegram/SourceFiles/mtproto/scheme/mtproto.tl@76225965bfc794388db241f3f760db8ab79e84f8`.
- Shipping source anchor remains
  `Telegram/SourceFiles/mtproto/session_private.cpp@13cbc76ec723386027b5b4dcac79999b791c4614`.
- Stage A may advance only to `implemented / validation_status=not-run / accepted=false`.
- Existing shared TL primitives remain the only parsing mechanism.
- `info:string` is binary state data at this fixed-U boundary; no lossy text conversion or UTF-8 gate is allowed.

## 10. Failure modes and edge cases

- Wrong constructor is accepted.
- `req_msg_id` is parsed with wrong width/endian or unsigned semantics.
- Binary state bytes such as `0xff` are rejected as invalid UTF-8 or normalized.
- Empty state payload is rejected.
- Short-form or 254-byte long-form TL length/padding is decoded incorrectly.
- TL header, payload or required padding truncation is accepted.
- Trailing bytes are ignored.
- The wire parser starts mutating request-history/application state.
- Exact parser support is used to claim arbitrary request-history, gzip, key-lifecycle or difference parity.
- Desktop contract mirror diverges from canonical authored contracts.

## 11. Implementation strategy

1. Commit this durable wire-parser spec before contract changes.
2. Preserve existing production api/tl/lib blobs because source review shows no production delta is required.
3. Extend the canonical external protocol contract with authored-only exact/binary/long-form/wrong-constructor/
   truncation/padding/trailing cases.
4. Mirror the canonical contract blob into the existing desktop tracked provider contract path.
5. Add one responsibility row to the existing parity ledger and synchronize existing P0/source-closure/evidence surfaces.
6. Perform only remote source/blob readback in Stage A; execute no validation.
7. Finish with a source-level compliance record distinguishing source/authored evidence from deferred Stage-B execution.

## 12. Verification / test strategy

Stage A: write and manually inspect contracts only.

Authored cases:
- exact constructor + `i64::MIN` + binary short-form TL string containing non-UTF-8 bytes → exact typed fields + EOF;
- exact constructor + 254-byte state payload → long-form TL framing preserved + EOF;
- wrong constructor → `UnexpectedConstructor`;
- constructor-only / truncated long / truncated TL header/body → existing `UnexpectedEof` path;
- state payload whose framing requires padding but omits padding → existing `UnexpectedEof` path;
- valid body plus one full TL word of trailing data → `InvalidMessageState`;
- empty state payload → accepted and exact EOF.

Stage B, only after full migration implementation closure: execute the canonical test/build/integration campaign
against the frozen candidate and bind evidence to that exact candidate SHA.

## 13. Acceptance criteria / Definition of Done

- AC-1: Existing `parse_msgs_state_info`, `TlReader` and public facade remain the only production wire owners and
  require no behavior delta.
- AC-2: Authored-only contracts lock R1-R7 and R9 without execution.
- AC-3: Desktop mirrors the canonical authored contract blob and keeps existing production api/lib unchanged.
- AC-4: Existing parity ledger/P0/source-closure/evidence record fixed-U identities and no-production-delta disposition.
- AC-5: New responsibility is `implemented / validation_status=not-run / accepted=false`.
- AC-6: No Stage-A executable validation or CI/status/check query is triggered, and no arbitrary request-history,
  connection-scheduling, auth-key-lifecycle, gzip or complete-difference claim is made.

## 14. Release / migration / rollback

Stage-A checkpoint branches only. No main integration, release, deployment, data migration or installation occurs.
Rollback is the immediately preceding checkpoint ref.

## 15. Observability / evidence

Only Git commit identities, blob SHAs, fixed-U source identities, authored test source and remote source readback are
produced in Stage A. No executable evidence is produced or claimed.

## 16. References / provenance

- `telegramdesktop/tdesktop@f985723b552b30fe1c17d377d57f44e61a15a8c4`
  - `Telegram/SourceFiles/mtproto/scheme/mtproto.tl@76225965bfc794388db241f3f760db8ab79e84f8`
  - `Telegram/SourceFiles/mtproto/session_private.cpp@13cbc76ec723386027b5b4dcac79999b791c4614`
- Fixed-U schema: `msgs_state_info#04deb57d req_msg_id:long info:string = MsgsStateInfo`.
- Fixed-U shipping: `SessionPrivate::handle` performs typed `MTPMsgsStateInfo::read(from,end)` before consulting
  `_stateAndResendRequests` and applying the returned state bytes.
- Existing Fabushi owner: `parse_msgs_state_info` + `TlReader::read_bytes`.

## 17. Spec compliance record

| Requirement / AC | Status | Evidence / reason |
| --- | --- | --- |
| R1-R8 | pending | Source readback complete; awaiting authored contract/bookkeeping save. |
| R9 | pending | Authored-only contract not yet saved. |
| R10 | pending | Awaiting final bookkeeping readback. |
| AC-1 | pending | Production blobs are unchanged at discovery time; final readback pending. |
| AC-2 | pending | Authored-only contract not yet saved. |
| AC-3 | pending | Desktop mirror not yet saved. |
| AC-4 | pending | Ledger/P0/source-closure/evidence update pending. |
| AC-5 | pending | Responsibility row not yet saved. |
| AC-6 | pending | Final Stage-A scope readback pending. |
