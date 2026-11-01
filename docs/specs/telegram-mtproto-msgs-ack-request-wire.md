# Telegram MTProto msgs_ack request wire contract

Status: active  
Owner: shared telegram-protocol request/TL codec  
Last updated: 2026-11-01  
Related issue/task/PR: Fabushi migration task 7457eb1c-11a3-4626-8563-a3e495872536 round 67

## 1. Context / problem

Fixed upstream U defines `msgs_ack#62d6b459 msg_ids:Vector<long> = MsgsAck`. In the shipping Telegram Desktop
`SessionPrivate::tryToSend` service-message path, a non-empty `_ackRequestData` vector is moved in caller order
into `MTP_vector<MTPlong>` and serialized as `MTP_msgs_ack`.

Fabushi already has the exact source-neutral builder in the existing canonical `telegram-protocol` owner:
`build_msgs_ack` writes the fixed-U constructor, delegates TL Vector framing/count conversion to the shared
`TlWriter`, and appends each signed 64-bit message id in input order. The desktop tracked provider has the same
canonical production api/lib blobs. Therefore this isolated request-wire responsibility requires no production
behavior delta.

The broader Telegram Desktop ACK lifecycle includes accumulated receive-message acknowledgement scheduling and
arbitrary sent/resend/container history. That lifecycle is not required to prove this pure serializer boundary and
remains outside this slice.

## 2. Goal

Close the exact fixed-U `msgs_ack#62d6b459` request serialization responsibility in the existing
`telegram-protocol` owner, with explicit authored-only vector-boundary contracts, without adding a second runtime,
provider, scheduler, request-history owner, or transport lifecycle.

## 3. Non-goals / out of scope

- No arbitrary historical sent/resend/container bookkeeping, callback history, ACK scheduling queue, or request settlement.
- No connection/TLS scheduling, timeout/backoff, reconnect, transport framing, or message-id generation changes.
- No auth-key binding, persistence, renewal, destruction, or teardown lifecycle.
- No `gzip_packed` decompression or complete updates difference/differenceSlice decoder/application.
- No change to existing inbound `parse_msgs_ack` or current single-request acknowledgement semantics.
- No Stage-A build, typecheck, lint, schema validation, generator execution, unit/contract/integration/E2E execution,
  Actions/status/check query, packaging, deployment, installation, or runtime acceptance.

## 4. Requirements

- R1: Preserve `MSGS_ACK_CONSTRUCTOR = 0x62d6b459`.
- R2: `build_msgs_ack(message_ids)` SHALL serialize: constructor, TL Vector constructor `0x1cb5c415`,
  signed 32-bit vector count, then each signed 64-bit `long` in caller-provided order.
- R3: The serializer SHALL not sort, deduplicate, filter, reinterpret, truncate, or otherwise mutate message ids.
- R4: Empty input SHALL serialize as a valid zero-length TL Vector. The wire-only builder SHALL not invent the
  shipping caller's independent non-empty scheduling condition.
- R5: Multi-element input SHALL be contiguous with no per-element constructor, padding, separator, or trailing bytes.
- R6: Signed `long` bit patterns SHALL be preserved exactly as little-endian 64-bit values.
- R7: TL Vector length conversion SHALL fail closed with `TlError::LengthOutOfRange` when `usize` cannot fit the
  TL signed `int` count, and the writer SHALL not emit a partial vector header on that failure.
- R8: Exact request serialization SHALL NOT be used to claim arbitrary ACK scheduling/history/application parity.
- R9: Canonical and desktop tracked contract surfaces SHALL contain authored-only cases covering zero, one, multiple
  signed values, exact EOF/order, and vector-length overflow/no-partial-header. They SHALL NOT be run in Stage A.

## 5. Current state

At platform-core checkpoint `06ac74bc56fcdb362e4e004980d6c60917311bf6`:

- `native/telegram-protocol/src/api.rs@ebfb3670b5cd097b9ce288782c7954f5cfe9fc43` defines the constructor,
  `build_msgs_ack`, and strict `parse_msgs_ack`.
- `build_msgs_ack` writes the constructor, invokes `TlWriter::write_vector_length(message_ids.len())`, then writes
  every `i64` in input order and returns exactly the accumulated bytes.
- `native/telegram-protocol/src/tl.rs@89256ab35184b6dc06a095ab34abb9f44e95a264` owns the canonical TL Vector
  constructor/count conversion and rejects lengths above signed `i32` before emitting the vector header.
- `native/telegram-protocol/src/lib.rs@abab39e42f100972484191f0e52e9edd9b88c6cf` exports the builder.
- Existing internal tests exercise a representative builder/parser round trip and trailing-byte parser rejection, but
  do not independently lock zero/multiple/signed exact request bytes and overflow/no-partial-header behavior.
- Desktop production api/lib are byte-identical to canonical production blobs.

## 6. Target state

Production api/tl/lib/network/runtime behavior remains unchanged. A durable wire-only spec plus authored-only external
protocol contracts independently lock the exact serializer/vector boundary. The desktop tracked contract mirror remains
byte-identical to the canonical contract file. The unique parity ledger records a separate request-wire responsibility as
implemented / validation_status=not-run / accepted=false.

## 7. Architecture and ownership boundaries

- `native/telegram-protocol/src/api.rs`: existing canonical `msgs_ack` builder.
- `native/telegram-protocol/src/tl.rs`: existing TL Vector framing/count owner.
- `native/telegram-protocol/tests/protocol_contract.rs`: canonical authored-only Stage-A contracts.
- Desktop tracked telegram-protocol contract path: existing consumer mirror only.
- ACK scheduling/history/application remains with existing runtime/network responsibilities; this slice owns no state.

No new owner or ADR is required.

## 8. Wire flow

`message_ids: &[i64]`
-> `build_msgs_ack`
-> `0x62D6B459` little-endian
-> `0x1CB5C415` TL Vector constructor
-> `count:i32` little-endian
-> `count * long:i64` little-endian in caller order
-> exact EOF.

## 9. Constraints

- Upstream U: `telegramdesktop/tdesktop@f985723b552b30fe1c17d377d57f44e61a15a8c4`.
- Schema blob: `Telegram/SourceFiles/mtproto/scheme/mtproto.tl@76225965bfc794388db241f3f760db8ab79e84f8`.
- Shipping source: `Telegram/SourceFiles/mtproto/session_private.cpp@13cbc76ec723386027b5b4dcac79999b791c4614`.
- Stage A may advance only to implemented / validation_status=not-run / accepted=false.

## 10. Failure modes

Wrong constructor/vector framing; wrong count width/endian; empty Vector rejected; ids reordered/deduplicated;
signed values narrowed; vector length wraps instead of failing; failure leaves partial vector header; request builder
is used to imply ACK scheduler/history parity; desktop contracts diverge from canonical.

## 11. Implementation strategy

1. Save this durable spec before contract changes.
2. Preserve existing production builder/TL/facade blobs because source review shows no production delta.
3. Add authored-only exact request contracts for empty, one, multiple signed values and direct vector overflow.
4. Mirror the canonical contract file to the existing desktop tracked provider contract path.
5. Add one responsibility row to the existing parity ledger and synchronize P0/source-closure/evidence.
6. Perform remote source/blob readback only; execute no validation.

## 12. Verification strategy

Stage A: author and manually inspect only:
- empty input -> constructor + Vector constructor + zero count + EOF;
- one id -> exact fixed-U bytes;
- multiple ids including signed extremes -> exact count/order/i64 preservation + EOF;
- direct TL Vector count overflow -> `TlError::LengthOutOfRange` and no partial writer bytes.

Stage B will execute these only after full implementation closure and candidate freeze.

## 13. Acceptance criteria

- AC-1: Existing `build_msgs_ack` and `TlWriter::write_vector_length` remain the only production wire owners.
- AC-2: Authored-only contracts lock R1-R7 without execution.
- AC-3: Desktop mirrors canonical authored contracts while production api/lib remain unchanged.
- AC-4: Unique ledger/P0/source-closure/evidence record fixed-U identities and no-production-delta disposition.
- AC-5: Responsibility is implemented / validation_status=not-run / accepted=false.
- AC-6: No Stage-A executable validation or Actions/status/check query; no arbitrary ACK history claim.

## 14. Release / rollback

Stage-A checkpoint branches only. No main integration, release, deployment, or installation occurs.
Rollback is the immediately preceding checkpoint ref.

## 15. Provenance

- Fixed-U `mtproto.tl`: `msgs_ack#62d6b459 msg_ids:Vector<long> = MsgsAck`.
- Fixed-U `SessionPrivate::tryToSend`: when `_ackRequestData` is non-empty, it serializes
  `MTP_msgs_ack(MTP_vector<MTPlong>(base::take(_ackRequestData)))`.
- Existing Fabushi owner: `build_msgs_ack` + `TlWriter::write_vector_length`.

## 16. Spec compliance record

Pending Stage-A authored-contract and bookkeeping commits. No executable validation is permitted in this round.
