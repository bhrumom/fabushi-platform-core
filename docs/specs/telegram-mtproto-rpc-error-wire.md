# Telegram MTProto rpc_error wire parser contract

Status: active  
Owner: shared telegram-protocol response/TL codec  
Last updated: 2026-11-01  
Related issue/task/PR: Fabushi migration task 7457eb1c-11a3-4626-8563-a3e495872536 round 52

## 1. Context / problem

Fixed upstream U defines `rpc_error#2144ca19 error_code:int error_message:string = RpcError`. In the shipping
Telegram Desktop `SessionPrivate::handle` service-message path, an `rpc_result#f35c6d01` body is queued as the
response payload; when its body constructor is `rpc_error`, Telegram Desktop treats it as an error response before
the request is handed to the main-thread request owner. The same path also contains broader request-history,
temporary-key and optional `gzip_packed` lifecycle behavior that is not required to prove the isolated RpcError wire
shape.

Fabushi already has a source-neutral typed parser in the existing shared `telegram-protocol` owner:
`try_parse_rpc_error` checks constructor `0x2144ca19`, reads `error_code:int`, reads the TL string, rejects malformed
TL input, and rejects trailing bytes. The public facade already exports the parser, response type and constructor.
No production behavior delta is required for this isolated response-wire responsibility.

The current external protocol contract has one positive code/message case, but does not independently lock
non-RpcError discrimination, truncation, invalid UTF-8 or trailing-byte rejection.

## 2. Goal

Close the fixed-U `rpc_error#2144ca19` exact typed parser/trailing-validation responsibility in the existing
`telegram-protocol` owner without creating a second Telegram runtime, request-history owner, decompressor or
auth-key lifecycle.

## 3. Non-goals / out of scope

- No arbitrary `wasSent/haveSent/_resendingIds/toSend/_sentContainers` request history or callback ownership.
- No request correlation, retry, cancellation, connection scheduling/racing, timeout/backoff or send lifecycle changes.
- No temporary/permanent auth-key binding, expiry, renewal, persistence, destruction or teardown.
- No `gzip_packed` decompression.
- No complete updates difference/differenceSlice decoder or application.
- No change to `rpc_result` request-history application or Telegram Desktop main-thread dispatch semantics.
- No Stage-A build, typecheck, lint, schema validation, generator execution, unit/contract/integration/E2E execution,
  Actions/status/check query, packaging, deployment, installation or runtime acceptance.

## 4. Requirements

- R1: Preserve `RPC_ERROR_CONSTRUCTOR = 0x2144ca19`.
- R2: `try_parse_rpc_error(input)` SHALL return `Ok(None)` for an input of at least four bytes whose leading
  constructor is not `RPC_ERROR_CONSTRUCTOR`, without interpreting the remaining body as RpcError.
- R3: For the RpcError constructor, the parser SHALL read exactly `error_code:int` as signed little-endian i32,
  followed by one canonical TL string, and return `RpcErrorResponse { code, message }`.
- R4: Constructor, int and TL-string truncation SHALL fail closed through the existing `TlError` path.
- R5: Invalid UTF-8 in `error_message:string` SHALL fail closed as `TlError::InvalidUtf8`.
- R6: After the TL string and its canonical padding, any remaining bytes SHALL fail closed as
  `ApiRequestError::TrailingBytes`.
- R7: Exact valid input SHALL terminate at EOF; the parser SHALL not normalize, classify or rewrite the machine
  error message at this wire boundary.
- R8: The desktop tracked provider SHALL remain a mirror of the canonical parser/facade/contracts; no second parser
  owner is introduced.
- R9: Canonical and desktop tracked contracts SHALL add authored-only positive and negative/boundary cases for exact
  fields, non-RpcError discrimination, truncation, invalid UTF-8 and trailing bytes. They SHALL NOT be executed in
  Stage A.
- R10: This wire slice SHALL NOT be used to claim arbitrary request-history, gzip, connection scheduling or auth-key
  lifecycle parity.

## 5. Current state

At platform-core checkpoint `be950b18eca7e3e6c1e9ffb4833a19fcbd224880`:

- `native/telegram-protocol/src/api.rs@ebfb3670b5cd097b9ce288782c7954f5cfe9fc43` defines
  `RPC_ERROR_CONSTRUCTOR = 0x2144ca19`, `RpcErrorResponse`, and `try_parse_rpc_error`.
- The parser first discriminates the four-byte constructor, then uses the existing `TlReader` to read i32 and
  `read_string()`, and rejects `!reader.is_finished()` with `TrailingBytes`.
- `native/telegram-protocol/src/tl.rs@89256ab35184b6dc06a095ab34abb9f44e95a264` owns canonical TL string
  length/padding, EOF and UTF-8 validation.
- `native/telegram-protocol/src/lib.rs@abab39e42f100972484191f0e52e9edd9b88c6cf` exports the same parser/type/
  constructor.
- `native/telegram-protocol/tests/protocol_contract.rs@89692a28620b2d8465703a1335458d5fbd15aee6` contains one positive
  `rpc_error_parser_preserves_code_and_machine_message` case only.
- Desktop tracked production api/lib and contract blobs are byte-identical to the canonical blobs above.

## 6. Target state

Production `api.rs`, `tl.rs`, `lib.rs`, networking and runtime behavior remain unchanged. The durable spec and
authored-only external contracts independently lock exact typed RpcError parsing and fail-closed trailing/malformed
behavior. Desktop mirrors the canonical authored contract file. Existing parity/P0/source-closure/evidence surfaces
record a separate no-production-delta responsibility as `implemented / validation_status=not-run / accepted=false`.

## 7. Architecture and ownership boundaries

- `native/telegram-protocol/src/api.rs`: existing canonical RpcError typed parser.
- `native/telegram-protocol/src/tl.rs`: existing TL int/string/padding/UTF-8/EOF primitive owner.
- `native/telegram-protocol/src/lib.rs`: existing source-neutral public facade.
- `native/telegram-protocol/tests/protocol_contract.rs`: canonical authored-only Stage-A wire contracts.
- Desktop tracked telegram-protocol source/contracts: existing consumer mirror only.
- Telegram Desktop request history, connection lifecycle, temporary-key handling and gzip: source behavior reference
  only; not recreated by this parser slice.

No new owner or ADR is required.

## 8. Interfaces / contracts / schemas / data flow

`input: &[u8]`
→ first 4 bytes
→ non-`0x2144CA19`: `Ok(None)`
→ `0x2144CA19`
→ `error_code:i32`
→ TL `error_message:string` with canonical length/padding/UTF-8
→ exact EOF
→ `Ok(Some(RpcErrorResponse { code, message }))`.

Fixed-U shipping reaches this object as the response body inside `rpc_result`. This parser neither owns nor
requires the broader request-id history that selected that response.

## 9. Constraints and non-functional requirements

- Upstream U remains `telegramdesktop/tdesktop@f985723b552b30fe1c17d377d57f44e61a15a8c4`.
- Schema blob remains `Telegram/SourceFiles/mtproto/scheme/mtproto.tl@76225965bfc794388db241f3f760db8ab79e84f8`.
- Shipping source anchor remains
  `Telegram/SourceFiles/mtproto/session_private.cpp@13cbc76ec723386027b5b4dcac79999b791c4614`.
- Stage A may advance only to `implemented / validation_status=not-run / accepted=false`.
- Existing shared TL primitives remain the only parsing mechanism.

## 10. Failure modes and edge cases

- Constructor is accepted when not `0x2144ca19`.
- Shorter-than-four-byte input is silently treated as non-error instead of malformed input.
- Signed error code is parsed with wrong width/endian.
- TL string short/long length, padding or body truncation is accepted.
- Invalid UTF-8 is lossily normalized instead of rejected.
- Trailing bytes are ignored.
- The wire parser mutates/classifies the machine message instead of preserving it.
- Exact parser support is used to claim request-history, gzip or key-lifecycle parity.
- Desktop contract mirror diverges from canonical authored contracts.

## 11. Implementation strategy

1. Commit this durable wire-parser spec before contract changes.
2. Preserve existing production api/tl/lib blobs because source review shows no production delta is required.
3. Extend the canonical external protocol contract with authored-only exact/non-match/truncation/UTF-8/trailing cases.
4. Mirror the canonical contract blob into the existing desktop tracked provider contract path.
5. Add one responsibility row to the existing parity ledger and synchronize existing P0/source-closure/evidence surfaces.
6. Perform only remote source/blob readback in Stage A; execute no validation.
7. Finish with a source-level compliance record distinguishing source/authored evidence from deferred Stage-B execution.

## 12. Verification / test strategy

Stage A: write and manually inspect contracts only.

Authored cases:
- exact constructor + signed code + TL string → typed response + exact EOF;
- different four-byte constructor → `Ok(None)`;
- constructor-only / truncated integer or string → existing `UnexpectedEof` path;
- invalid UTF-8 TL string → `InvalidUtf8`;
- valid RpcError plus one full TL word of trailing data → `TrailingBytes(4)`.

Stage B, only after full migration implementation closure: execute the canonical test/build/integration campaign
against the frozen candidate and bind evidence to that exact candidate SHA.

## 13. Acceptance criteria / Definition of Done

- AC-1: Existing `try_parse_rpc_error`, `TlReader` and public facade remain the only production wire owners and
  require no behavior delta.
- AC-2: Authored-only contracts lock R1-R7 and R9 without execution.
- AC-3: Desktop mirrors the canonical authored contract blob and keeps existing production api/lib unchanged.
- AC-4: Existing parity ledger/P0/source-closure/evidence record fixed-U identities and no-production-delta disposition.
- AC-5: New responsibility is `implemented / validation_status=not-run / accepted=false`.
- AC-6: No Stage-A executable validation or CI/status/check query is triggered, and no arbitrary request-history,
  gzip, connection-scheduling or auth-key-lifecycle claim is made.

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
- Fixed-U schema: `rpc_error#2144ca19 error_code:int error_message:string = RpcError`.
- Fixed-U shipping: `SessionPrivate` handles `mtpc_rpc_result`, identifies `mtpc_rpc_error` in the response body,
  and queues that body for the existing request owner; gzip and request-history branches remain outside this slice.
- Existing Fabushi owner: `try_parse_rpc_error` + `TlReader`.

## 17. Spec compliance record

| Requirement / AC | Status | Evidence / reason |
| --- | --- | --- |
| R1-R8 | passed | Remote source readback at platform checkpoint 7ad951b93b57fc3c4cf33e43eaa157066d452b31: api.rs@ebfb3670b5cd097b9ce288782c7954f5cfe9fc43 preserves constructor 0x2144ca19, typed signed i32 + TL string parsing, non-RpcError discrimination and strict trailing-byte rejection; tl.rs@89256ab35184b6dc06a095ab34abb9f44e95a264 preserves canonical TL string padding, truncation and UTF-8 validation; lib.rs@abab39e42f100972484191f0e52e9edd9b88c6cf exports the same parser/type/constructor. Fixed-U rpc_result handling carries rpc_error as the response body, while request history, gzip and auth-key lifecycle remain outside this slice. |
| R9 | passed | Canonical authored-only contract file is native/telegram-protocol/tests/protocol_contract.rs@d171a3ab9a43955a693c87162f80c2667fc397ce; desktop mirror commit 885faac55fc8b7d99e4989b438e1a428ac57c1f8 has the same blob. Cases cover exact fields, non-RpcError discrimination, truncation, invalid UTF-8, trailing bytes and EOF. They were not executed. |
| R10 | passed | Ledger/evidence keep arbitrary request history, connection scheduling, auth-key binding/persistence, gzip and complete difference decoding explicitly open/fail-closed. |
| AC-1 | passed | Production api/tl/lib blobs remain ebfb3670b5cd097b9ce288782c7954f5cfe9fc43 / 89256ab35184b6dc06a095ab34abb9f44e95a264 / abab39e42f100972484191f0e52e9edd9b88c6cf; no production behavior delta was made. |
| AC-2 | passed | Authored contract commit 7ad951b93b57fc3c4cf33e43eaa157066d452b31 and blob d171a3ab9a43955a693c87162f80c2667fc397ce lock R1-R7/R9; executable validation is intentionally deferred. |
| AC-3 | passed | Desktop production api/lib remain ebfb3670b5cd097b9ce288782c7954f5cfe9fc43 / abab39e42f100972484191f0e52e9edd9b88c6cf; desktop contract mirror is byte-identical at d171a3ab9a43955a693c87162f80c2667fc397ce. |
| AC-4 | passed | Desktop ledger/evidence/P0/source-closure commit 0c9547557a9b69734cedae9ec7d2fcf7607fa0ce records fixed-U identities, no-production-delta disposition and open broader lifecycle boundaries. |
| AC-5 | passed | Existing parity ledger records TDRP-R9-MTPROTO-RPC-ERROR-WIRE-001 as implemented / validation_status=not-run / accepted=false with responsibilities_total=93, unknown=16172, unread=16145, omitted=0. |
| AC-6 | passed | This Stage-A round performed source/blob readback and authored/saved code/docs only. No build, typecheck, lint, schema validator, generator, unit/contract/integration/E2E execution, GitHub Actions/status/check query, package, deploy, install or runtime acceptance was performed. |

Source-level compliance is complete for this isolated response-wire slice. This does not constitute executable
verification, candidate readiness, release acceptance, or closure of the broader migration.
