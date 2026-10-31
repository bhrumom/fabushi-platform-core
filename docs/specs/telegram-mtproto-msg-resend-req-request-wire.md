# Telegram MTProto msg_resend_req request wire contract

Status: active  
Owner: shared telegram-protocol request/TL codec  
Last updated: 2026-11-01  
Related issue/task/PR: Fabushi migration task 7457eb1c-11a3-4626-8563-a3e495872536 round 51

## 1. Context / problem

Fixed upstream U defines `msg_resend_req#7d861a08 msg_ids:Vector<long> = MsgResendReq`. In the shipping
Telegram Desktop `SessionPrivate::tryToSend` service-message path, a non-empty `_resendRequestData`
vector is moved into `MTP_vector<MTPlong>` and serialized as `MTP_msg_resend_req`. Fixed-U also
populates that vector when detailed-answer metadata says an answer message was not received.

Fabushi already has an exact source-neutral request builder: `build_msg_resend_req` writes the fixed-U
constructor, delegates TL Vector framing/count conversion to the shared `TlWriter`, and appends each signed
64-bit message id in caller order. The same builder is exported by the existing telegram-protocol facade and
mirrored in the desktop tracked provider. No production behavior delta is required for this isolated request-wire
responsibility.

The shipping source's broader resend lifecycle relies on arbitrary `haveSent`, `_resendingIds`,
`toSend`, `_sentContainers`, request callback history, connection scheduling and retry behavior. Those are
not prerequisites for proving this pure serializer boundary and remain explicitly outside this slice.

## 2. Goal

Close the fixed-U `msg_resend_req#7d861a08` request serialization responsibility in the existing
`telegram-protocol` owner, with exact TL Vector boundary coverage, without creating another Telegram
runtime/provider or claiming arbitrary multi-request resend-history parity.

## 3. Non-goals / out of scope

- No implementation or claim of arbitrary `haveSent`, `_resendingIds`, `toSend`, `_sentContainers`,
  callback registry, resend queue, container retry, or multi-request history.
- No connection scheduling/racing, timeout/backoff, message-id generation, send retry, or transport lifecycle changes.
- No temporary/permanent auth-key binding, expiry, renewal, persistence, destruction, or teardown.
- No `gzip_packed` decompression or complete updates difference/differenceSlice decoder/application.
- No changes to detailed-info receive/application semantics beyond citing the existing fixed-U source trigger.
- No Stage-A build, typecheck, lint, schema validation, generator execution, unit/contract/integration/E2E execution,
  Actions/status/check query, packaging, deployment, installation, or runtime acceptance.

## 4. Requirements

- R1: Preserve `MSG_RESEND_REQ_CONSTRUCTOR = 0x7d861a08`.
- R2: `build_msg_resend_req(message_ids)` SHALL serialize fields in fixed-U order:
  constructor, TL Vector constructor `0x1cb5c415`, signed 32-bit vector count, then each signed
  64-bit `long` value in the caller-provided order.
- R3: The serializer SHALL not sort, deduplicate, filter, reinterpret, truncate, or otherwise mutate message ids.
- R4: An empty input slice SHALL serialize as a valid zero-length TL Vector. The pure wire builder SHALL not invent
  a non-empty semantic restriction absent from the TL schema; the shipping caller independently sends only when its
  resend vector is non-empty.
- R5: Multi-element input SHALL be serialized contiguously with no per-element constructor, padding, separator, or
  trailing bytes after the final `long`.
- R6: Signed `long` bit patterns SHALL be preserved exactly as little-endian 64-bit values. This wire responsibility
  imposes no resend/history policy on those ids.
- R7: TL Vector length conversion SHALL fail closed with `TlError::LengthOutOfRange` when a `usize` length cannot
  be represented as the TL signed `int` count; the shared writer SHALL not emit a partial vector header on failure.
- R8: The fixed-U shipping resend-history lifecycle SHALL remain unclaimed by this slice. Exact request serialization
  support SHALL NOT be used to represent or imply arbitrary multi-request resend-state parity.
- R9: Canonical and desktop tracked contract surfaces SHALL contain authored-only positive/boundary cases for
  zero, one, multiple signed longs, exact EOF, stable ordering, and vector-length overflow. They SHALL NOT be run in Stage A.

## 5. Current state

At platform-core checkpoint `f2593977cbdfe1a09ce811da86029ec824021af3`:

- `native/telegram-protocol/src/api.rs@ebfb3670b5cd097b9ce288782c7954f5cfe9fc43` defines
  `MSG_RESEND_REQ_CONSTRUCTOR = 0x7d861a08` and `build_msg_resend_req`.
- The builder writes the constructor, calls `TlWriter::write_vector_length(message_ids.len())`, writes each
  `i64` in order, and returns exactly the accumulated bytes.
- `native/telegram-protocol/src/tl.rs@89256ab35184b6dc06a095ab34abb9f44e95a264` writes the canonical
  Vector constructor and rejects lengths that cannot convert to signed `i32` before writing the vector header.
- `native/telegram-protocol/src/lib.rs@abab39e42f100972484191f0e52e9edd9b88c6cf` exports the same builder.
- Existing internal API coverage checks one id, but the external protocol contract surface does not yet independently
  lock empty/multiple/signed ordering and overflow behavior for `msg_resend_req`.
- Desktop tracked production api/lib blobs are byte-identical to canonical production blobs.

## 6. Target state

Production `api.rs`, `tl.rs`, `lib.rs`, networking and runtime behavior remain unchanged. A dedicated durable
spec and authored-only contract cases independently lock the exact fixed-U serializer/vector boundary. The desktop
provider contract mirror remains byte-identical to canonical authored contracts. Existing parity/P0/source-closure
bookkeeping records a separate no-production-delta responsibility as implemented / validation_status=not-run /
accepted=false.

## 7. Architecture and ownership boundaries

- `native/telegram-protocol/src/api.rs`: existing canonical `msg_resend_req` request builder.
- `native/telegram-protocol/src/tl.rs`: existing TL Vector framing/count conversion owner.
- `native/telegram-protocol/tests/protocol_contract.rs`: canonical authored-only Stage-A wire contracts.
- Desktop tracked telegram-protocol source/contracts: existing consumer mirror only.
- Telegram Desktop resend/history state: source behavior reference only; not recreated by this serializer slice.

No new owner or ADR is required.

## 8. Interfaces / contracts / schemas / data flow

`message_ids: &[i64]`
→ `build_msg_resend_req`
→ `0x7D861A08` little-endian
→ `0x1CB5C415` TL Vector constructor
→ `count:i32` little-endian
→ `count * long:i64` little-endian in caller order
→ exact EOF.

Fixed-U shipping only constructs the service message when `_resendRequestData` is non-empty. Pure builder support
for arbitrary schema-valid Vector cardinality does not create a runtime resend-history owner.

## 9. Constraints and non-functional requirements

- Upstream U remains `telegramdesktop/tdesktop@f985723b552b30fe1c17d377d57f44e61a15a8c4`.
- Schema blob remains `Telegram/SourceFiles/mtproto/scheme/mtproto.tl@76225965bfc794388db241f3f760db8ab79e84f8`.
- Shipping source anchor remains `Telegram/SourceFiles/mtproto/session_private.cpp@13cbc76ec723386027b5b4dcac79999b791c4614`.
- Stage A may advance only to implemented / validation_status=not-run / accepted=false.
- Existing shared TL codec semantics remain the only serialization primitive.

## 10. Failure modes and edge cases

- Wrong constructor is emitted.
- Vector constructor is omitted or wrong.
- Vector count uses wrong width/endian/sign representation.
- Empty Vector is rejected despite the TL schema permitting count zero.
- Multi-element ids are reordered, deduplicated, filtered, narrowed, or individually framed.
- Signed 64-bit values are reinterpreted.
- A vector length above `i32::MAX` wraps/truncates instead of failing.
- Length overflow leaves a partial Vector header.
- The wire-only builder is used to claim Telegram Desktop resend-history lifecycle parity.
- Desktop contract mirror diverges from canonical protocol contracts.

## 11. Implementation strategy

1. Commit this durable request-wire spec before any contract change.
2. Preserve existing production builder/TL/facade blobs because source review shows no production delta is required.
3. Add authored-only exact external protocol contracts for zero, one and multiple signed values plus overflow behavior.
4. Mirror the canonical contract file into the existing desktop tracked provider contract path.
5. Add one responsibility row to the existing parity ledger and synchronize existing P0/source-closure/evidence surfaces.
6. Perform only remote source/blob readback in Stage A; run no executable validation.
7. Finish with a source-level compliance record distinguishing source/authored evidence from deferred Stage-B execution.

## 12. Verification / test strategy

Stage A: write and manually inspect contracts only.

Authored cases:
- empty input → constructor + Vector constructor + zero count + EOF;
- one shipping-shaped id → exact fixed-U bytes;
- multiple ids including negative/zero/positive extremes → exact count/order/64-bit preservation + EOF;
- direct TL Vector count overflow → `TlError::LengthOutOfRange` and no partial writer bytes.

Stage B, only after full migration implementation closure: execute the canonical test/build/integration campaign against
the frozen candidate and bind evidence to that exact candidate SHA.

## 13. Acceptance criteria / Definition of Done

- AC-1: Existing `build_msg_resend_req` and `TlWriter::write_vector_length` remain the only production wire owners
  and require no behavior delta.
- AC-2: Authored-only contracts lock R1-R7 without execution.
- AC-3: Desktop mirrors the canonical authored contract blob and keeps existing production api/lib facade unchanged.
- AC-4: Existing parity ledger/P0/source-closure/evidence record fixed-U source identities and no-production-delta disposition.
- AC-5: New responsibility is implemented / validation_status=not-run / accepted=false.
- AC-6: No Stage-A executable validation or CI/status/check query is triggered, and no multi-request lifecycle claim is made.

## 14. Release / migration / rollback

Stage-A checkpoint branches only. No main integration, release, deployment, data migration, or installation occurs.
Rollback is the immediately preceding checkpoint ref.

## 15. Observability / evidence

Only Git commit identities, blob SHAs, fixed-U source identities, authored test source and remote source readback are
produced in Stage A. No executable evidence is produced or claimed.

## 16. References / provenance

- `telegramdesktop/tdesktop@f985723b552b30fe1c17d377d57f44e61a15a8c4`
  - `Telegram/SourceFiles/mtproto/scheme/mtproto.tl@76225965bfc794388db241f3f760db8ab79e84f8`
  - `Telegram/SourceFiles/mtproto/session_private.cpp@13cbc76ec723386027b5b4dcac79999b791c4614`
- Shipping source: `SessionPrivate::tryToSend` constructs
  `MTP_msg_resend_req(MTP_vector<MTPlong>(base::take(_resendRequestData)))` only for a non-empty vector.
- Existing Fabushi owner: `build_msg_resend_req` + `TlWriter::write_vector_length`.

## 17. Spec compliance record

| Requirement / AC | Status | Evidence / reason |
| --- | --- | --- |
| R1-R8 | passed | Remote source readback at platform checkpoint 161d78f68715540e733c43895aba2f1be4619755: api.rs@ebfb3670b5cd097b9ce288782c7954f5cfe9fc43 preserves constructor 0x7d861a08 and exact constructor/Vector/count/i64 order; tl.rs@89256ab35184b6dc06a095ab34abb9f44e95a264 preserves canonical Vector framing plus usize-to-i32 overflow failure before header emission; lib.rs@abab39e42f100972484191f0e52e9edd9b88c6cf exports the same builder. Fixed-U SessionPrivate::tryToSend emits msg_resend_req only for non-empty _resendRequestData, while arbitrary resend-history lifecycle remains explicitly outside this slice. |
| R9 | passed | Canonical authored-only contract file is native/telegram-protocol/tests/protocol_contract.rs@89692a28620b2d8465703a1335458d5fbd15aee6 from commit 161d78f68715540e733c43895aba2f1be4619755; desktop mirror commit 37299e18e3018e988d541607d5ba7842713d9fd1 has the same blob. Cases cover zero, one, multiple signed longs, exact EOF and vector-count overflow/no-partial-header. They were not executed. |
| AC-1 | passed | Production api/tl/lib blobs remain ebfb3670b5cd097b9ce288782c7954f5cfe9fc43 / 89256ab35184b6dc06a095ab34abb9f44e95a264 / abab39e42f100972484191f0e52e9edd9b88c6cf; no production behavior delta was made. |
| AC-2 | passed | Authored contract commit 161d78f68715540e733c43895aba2f1be4619755 and blob 89692a28620b2d8465703a1335458d5fbd15aee6 lock R1-R7; executable validation is intentionally deferred. |
| AC-3 | passed | Desktop production api remains ebfb3670b5cd097b9ce288782c7954f5cfe9fc43, desktop facade remains abab39e42f100972484191f0e52e9edd9b88c6cf, and desktop contract mirror is byte-identical at 89692a28620b2d8465703a1335458d5fbd15aee6. |
| AC-4 | passed | Desktop ledger/evidence commit 5cd36cdd14e7321b0d96526f334b1f97e4243510 and P0/source-closure sync commit f0500022cdb18c66f4d2dbef98479c77e0bea61f record fixed-U identities, no-production-delta disposition and open broader lifecycle boundaries. |
| AC-5 | passed | Existing parity ledger records TDRP-R9-MTPROTO-MSG-RESEND-REQ-REQUEST-WIRE-001 as implemented / validation_status=not-run / accepted=false with responsibilities_total=92, unknown=16172, unread=16145, omitted=0. |
| AC-6 | passed | This Stage-A round performed source/blob readback and authored/saved code/docs only. No build, typecheck, lint, schema validator, generator, unit/contract/integration/E2E execution, GitHub Actions/status/check query, package, deploy, install or runtime acceptance was performed; arbitrary multi-request history remains open/fail-closed. |

Source-level compliance is complete for this isolated request-wire slice. This does not constitute executable
verification, candidate readiness, release acceptance, or closure of the broader migration.
