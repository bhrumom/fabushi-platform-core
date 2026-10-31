# Telegram MTProto msgs_state_req request wire contract

Status: active  
Owner: shared telegram-protocol request/TL codec  
Last updated: 2026-11-01  
Related issue/task/PR: Fabushi migration task 7457eb1c-11a3-4626-8563-a3e495872536 round 50

## 1. Context / problem

Fixed upstream U defines `msgs_state_req#da69fb52 msg_ids:Vector<long> = MsgsStateReq`. In the shipping
Telegram Desktop `SessionPrivate::tryToSend` service-message path, non-empty `_stateRequestData` is copied
in order into `MTP_vector<MTPlong>` and serialized as `MTP_msgs_state_req`. The later
`mtpc_msgs_state_info` path parses the previously sent request to recover the same id vector before
`handleMsgsStates`.

Fabushi already implemented the exact builder during the earlier single-outstanding-RPC state-reconciliation
slice: `build_msgs_state_req` writes the fixed-U constructor, uses the shared TL Vector writer, and appends
each signed 64-bit message id in input order. The current desktop tracked provider has the same canonical
`api.rs` blob. No production behavior delta is required for this independent request-wire responsibility.
The remaining Stage-A gap is a durable wire-only spec, explicit vector-boundary contracts, desktop contract
mirroring, and migration bookkeeping.

## 2. Goal

Close the fixed-U `msgs_state_req#da69fb52` request serialization responsibility in the existing
`telegram-protocol` owner without creating another service-message/runtime/provider owner or expanding
Fabushi's current single-outstanding-RPC lifecycle into Telegram Desktop's arbitrary sent/resend history.

## 3. Non-goals / out of scope

- No new multi-request sent history, `haveSent`, `_resendingIds`, `_stateAndResendRequests`, resend queue,
  simultaneous state-query scheduler, or container-wide retry bookkeeping.
- No connection scheduling/racing, timeout/backoff policy, plaintext transport lifecycle, or message-id generation changes.
- No temporary/permanent auth-key binding, expiry, renewal, persistence, destruction, or teardown.
- No `gzip_packed` decompression or complete updates difference/differenceSlice decoder/application.
- No change to the existing `msgs_state_info` / `msgs_all_info` parser or single-RPC reconciliation semantics.
- No Stage-A build, typecheck, lint, schema validation, generator execution, unit/contract/integration/E2E execution,
  Actions/status/check query, packaging, deployment, installation, or runtime acceptance.

## 4. Requirements

- R1: Preserve `MSGS_STATE_REQ_CONSTRUCTOR = 0xda69fb52`.
- R2: `build_msgs_state_req(message_ids)` SHALL serialize fields in fixed-U order:
  constructor, `Vector<long>` constructor `0x1cb5c415`, signed 32-bit vector count, then each signed
  64-bit `long` value in the caller-provided order.
- R3: The serializer SHALL not sort, deduplicate, filter, reinterpret, truncate, or otherwise mutate message ids.
- R4: An empty input slice SHALL serialize as a valid zero-length TL Vector. The wire-only builder SHALL not invent
  a semantic non-empty restriction that is absent from the TL schema; the current shipping caller independently
  sends only when its state-request set is non-empty.
- R5: Multi-element input SHALL be serialized contiguously with no per-element constructor, padding, or separator,
  and the result SHALL contain no trailing bytes after the final `long`.
- R6: Signed `long` bit patterns SHALL be preserved exactly as little-endian 64-bit values. This wire responsibility
  does not impose a message-id policy beyond the existing caller/runtime owner.
- R7: TL Vector length conversion SHALL fail closed with `TlError::LengthOutOfRange` when a `usize` length cannot
  be represented as the TL signed `int` count; the writer SHALL not emit a partial vector header on that failure.
- R8: The live Fabushi state-reconciliation path SHALL remain the existing single-outstanding-RPC owner. This slice
  validates only exact request bytes and SHALL NOT claim arbitrary multi-request lifecycle parity.
- R9: Canonical and desktop tracked contract surfaces SHALL contain authored-only positive/boundary cases for
  zero, one, multiple signed longs, exact EOF, stable ordering, and vector length overflow. They SHALL NOT be run in Stage A.

## 5. Current state

At platform-core checkpoint `26cc521c1e57944a9f27fa16f469fff870f34517`:

- `native/telegram-protocol/src/api.rs@ebfb3670b5cd097b9ce288782c7954f5cfe9fc43` already defines
  `MSGS_STATE_REQ_CONSTRUCTOR = 0xda69fb52` and `build_msgs_state_req`.
- The builder writes the constructor, calls `TlWriter::write_vector_length(message_ids.len())`, writes each
  `i64` in order, and returns exactly the accumulated bytes.
- `native/telegram-protocol/src/tl.rs@89256ab35184b6dc06a095ab34abb9f44e95a264` writes the canonical Vector
  constructor and rejects lengths that cannot convert to signed `i32`.
- Existing internal API coverage checks one single-id request, but the canonical external protocol contract surface
  does not yet independently lock zero/multiple/signed ordering and vector-overflow behavior.
- Desktop tracked `source/mahayana/mahayana-rs/providers/telegram-protocol/src/api.rs` is byte-identical to the
  canonical api blob `ebfb3670b5cd097b9ce288782c7954f5cfe9fc43`.

## 6. Target state

Production `api.rs`, `tl.rs`, `lib.rs`, and runtime behavior remain unchanged. A dedicated durable spec and
authored-only contract cases independently prove the exact fixed-U serializer/vector boundary; the existing desktop
provider contract mirror remains byte-identical to the canonical authored contract file. The parity ledger records
a separate request-wire responsibility as implemented / validation_status=not-run / accepted=false.

## 7. Architecture and ownership boundaries

- `native/telegram-protocol/src/api.rs`: existing canonical `msgs_state_req` request builder.
- `native/telegram-protocol/src/tl.rs`: existing TL Vector framing/count conversion owner.
- `native/telegram-protocol/tests/protocol_contract.rs`: canonical authored-only Stage-A wire contracts.
- Desktop tracked telegram-protocol source/contracts: existing consumer mirror only.
- `telegram-network::EstablishedSession`: unchanged single-current-RPC lifecycle owner; no broader history is added.

No new owner or ADR is required.

## 8. Interfaces / contracts / schemas / data flow

`message_ids: &[i64]`
→ `build_msgs_state_req`
→ `0xDA69FB52` little-endian
→ `0x1CB5C415` TL Vector constructor
→ `count:i32` little-endian
→ `count * long:i64` little-endian in caller order
→ exact EOF.

The shipping current-RPC path continues to call this builder with one current request message id through the existing
state-reconciliation flow. Pure builder support for a TL Vector does not create or imply a multi-request runtime history.

## 9. Constraints and non-functional requirements

- Upstream U remains `telegramdesktop/tdesktop@f985723b552b30fe1c17d377d57f44e61a15a8c4`.
- Schema blob remains `Telegram/SourceFiles/mtproto/scheme/mtproto.tl@76225965bfc794388db241f3f760db8ab79e84f8`.
- Shipping source anchor remains `Telegram/SourceFiles/mtproto/session_private.cpp@13cbc76ec723386027b5b4dcac79999b791c4614`.
- Stage A may advance only to implemented / validation_status=not-run / accepted=false.
- Existing shared TL codec semantics remain the only serialization primitive.

## 10. Failure modes and edge cases

- Wrong constructor is emitted.
- Vector constructor is omitted or wrong.
- Vector count uses the wrong width/endian/sign representation.
- Empty Vector is rejected despite the TL schema allowing count zero.
- Multi-element ids are reordered, deduplicated, filtered, or padded individually.
- Signed 64-bit values are narrowed or reinterpreted.
- A vector length above `i32::MAX` wraps/truncates instead of failing.
- Length overflow leaves a partial Vector header in a reusable writer.
- The wire-only builder is used to claim arbitrary multi-request sent-history parity.
- Desktop diverges from the canonical protocol contract surface.

## 11. Implementation strategy

1. Commit this durable request-wire spec before contract changes.
2. Preserve existing production builder/TL/runtime blobs because source review shows no production delta is required.
3. Add exact authored-only external protocol contracts for zero, one and multiple signed values plus overflow behavior.
4. Mirror the canonical contract file into the existing desktop tracked provider contract path.
5. Add one responsibility row to the existing parity ledger and synchronize the existing P0/source-closure/evidence surfaces.
6. Perform only remote source/blob readback in Stage A; run no executable validation.
7. Finish with a source-level compliance record that distinguishes authored evidence from deferred Stage-B execution.

## 12. Verification / test strategy

Stage A: write and manually inspect contracts only.

Authored cases:
- empty input → constructor + Vector constructor + zero count + EOF;
- one id → exact fixed-U bytes;
- multiple ids including negative/zero/positive extremes → exact count/order/64-bit preservation + EOF;
- direct TL Vector count overflow → `TlError::LengthOutOfRange` and no partial writer bytes.

Stage B, only after full migration implementation closure: execute the canonical test/build/integration campaign against
the frozen candidate and bind evidence to that exact candidate SHA.

## 13. Acceptance criteria / Definition of Done

- AC-1: Existing `build_msgs_state_req` and `TlWriter::write_vector_length` remain the only production wire owners
  and require no behavior delta.
- AC-2: Authored-only contracts lock R1-R7 without execution.
- AC-3: Desktop mirrors the canonical authored contract blob and keeps its existing production api/lib facade unchanged.
- AC-4: Existing parity ledger/P0/source-closure/evidence record fixed-U source identities and the no-production-delta disposition.
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
- Shipping source: `SessionPrivate::tryToSend` constructs `MTP_msgs_state_req(MTP_vector<MTPlong>(ids))`.
- Existing Fabushi owner: `build_msgs_state_req` + `TlWriter::write_vector_length`.

## 17. Spec compliance record

| Requirement / AC | Status | Evidence / reason |
| --- | --- | --- |
| R1-R8 | pending | Final source-level readback is recorded after authored contracts/bookkeeping are saved. |
| R9 | pending | Canonical and desktop mirrored contracts are authored after this spec commit and remain unrun. |
| AC-1 | pending | Final source-level production blob readback required. |
| AC-2 | pending | Authored contract boundary matrix not yet committed at spec creation time. |
| AC-3 | pending | Desktop mirror not yet committed at spec creation time. |
| AC-4 | pending | Ledger/P0/source-closure/evidence sync not yet committed at spec creation time. |
| AC-5 | pending | Ledger row not yet committed at spec creation time. |
| AC-6 | pending | Final source-level timing/scope review required. |
