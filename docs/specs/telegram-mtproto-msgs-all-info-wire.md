# Telegram MTProto msgs_all_info wire responsibility

Status: Stage A implementation bookkeeping; executable validation intentionally deferred.
Task: TDRP-001 / task 7457eb1c-11a3-4626-8563-a3e495872536 / round 54.

## 1. Purpose

Close only the fixed-U service-message wire responsibility for
`msgs_all_info#8cc0d131 msg_ids:Vector<long> info:string = MsgsAllInfo` inside the existing source-neutral
`telegram-protocol` owner.

This slice is deliberately separate from Telegram Desktop arbitrary request-history/application semantics.
It does not migrate `haveSent`, `_resendingIds`, `toSend`, `_sentContainers`, resend scheduling,
connection scheduling/racing/send retry, auth-key binding/persistence, gzip, or complete difference decoding/application.

## 2. Fixed source authority

Fixed upstream U remains:
`telegramdesktop/tdesktop@f985723b552b30fe1c17d377d57f44e61a15a8c4`.

Relevant fixed-U identities:
- `Telegram/SourceFiles/mtproto/scheme/mtproto.tl@76225965bfc794388db241f3f760db8ab79e84f8`
- `Telegram/SourceFiles/mtproto/session_private.cpp@13cbc76ec723386027b5b4dcac79999b791c4614`

The schema declares:
`msgs_all_info#8cc0d131 msg_ids:Vector<long> info:string = MsgsAllInfo`.

Shipping `SessionPrivate::handle` first calls `MTPMsgsAllInfo::read(from,end)`, obtains the typed
`msg_ids` vector and binary `info` bytes, then calls `handleMsgsStates(ids, states)`.
`handleMsgsStates` checks `states.size() == ids.size()` before consulting sent/resend history.
That history lookup, replay, acknowledgement and scheduling behavior is outside this wire-only slice.

## 3. Existing owner disposition

No production behavior delta is required.

Current canonical production owner:
- `native/telegram-protocol/src/api.rs@ebfb3670b5cd097b9ce288782c7954f5cfe9fc43`
  - `MSGS_ALL_INFO_CONSTRUCTOR = 0x8cc0d131`
  - `MsgsAllInfo`
  - `parse_msgs_all_info`
- `native/telegram-protocol/src/tl.rs@89256ab35184b6dc06a095ab34abb9f44e95a264`
  - `TlReader::read_vector_length`
  - `TlReader::read_i64`
  - `TlReader::read_bytes`
- `native/telegram-protocol/src/lib.rs@abab39e42f100972484191f0e52e9edd9b88c6cf`
  - public export of the same parser/type/constructor.

`parse_msgs_all_info` already:
1. requires constructor `0x8cc0d131`;
2. requires the canonical TL vector constructor;
3. rejects negative vector lengths and vector lengths above the existing 1,000,000 element bound;
4. reads every `long` as signed little-endian `i64`;
5. preserves `info:string` as opaque binary bytes through `read_bytes`, rather than UTF-8 text;
6. enforces TL byte-string short/long framing, configured length limits, required padding consumption and truncation failure;
7. rejects state/vector cardinality mismatch before any request-history application;
8. requires exact EOF and rejects trailing input.

The state/vector cardinality check is a fail-closed state-shape boundary, matching the fixed-U
`handleMsgsStates` precondition before history access. It does not make the parser the owner of request history.

## 4. Requirements

- R1: Constructor must be exactly `0x8cc0d131`.
- R2: `msg_ids` must use the canonical TL vector constructor.
- R3: Vector count must reject negative values and values above the existing bounded-reader limit.
- R4: Each message id must preserve signed `long` / `i64` semantics and little-endian wire order.
- R5: `info:string` is binary state data and must preserve arbitrary non-UTF-8 bytes.
- R6: TL bytes/string short form, 254-byte long form, truncation and required padding must remain fail-closed.
- R7: State byte count must equal message-id count before returning a typed value.
- R8: Valid bodies must end at exact EOF; trailing data must fail closed.
- R9: Parsing must be side-effect free and must not consult or mutate arbitrary request history/application state.
- R10: Stage A may author contracts and bookkeeping only; no build, test, validator, Actions/status/check query,
  package, deploy, install or runtime acceptance is executed.

## 5. Failure behavior

Expected fail-closed cases include:
- wrong top-level constructor;
- wrong vector constructor;
- negative vector count;
- vector count above the configured limit;
- truncated vector count or message-id body;
- malformed/truncated TL bytes header or body;
- missing TL bytes padding;
- state byte count different from message-id count;
- valid typed body followed by trailing bytes.

No failure path may silently route into a second Telegram parser/runtime or claim request-history parity.

## 6. Existing-owner-first architecture

Existing owner candidates:
- canonical shared `native/telegram-protocol`;
- desktop tracked `source/mahayana/mahayana-rs/providers/telegram-protocol` mirror.

Rejected owners:
- `telegram-network` or runtime request-history owner for this pure wire parse;
- a second Telegram service-message parser;
- a desktop-only parser that diverges from the canonical shared protocol owner.

No new owner or ADR is required.

## 7. Production composition

The production composition remains:
existing account/session lifecycle -> embedded MTProto transport -> existing shared telegram-protocol wire parser.

This slice does not add a provider, sidecar, subprocess, second session system, second request-history store,
second message store, or new UI surface.

## 8. Authored-only contract plan

Stage A contracts must be written but not executed. They must cover:
- signed message-id extremes plus non-UTF-8 binary short-form state bytes and exact EOF;
- empty vector + empty state;
- 254 message ids + 254 state bytes to exercise long-form TL bytes framing;
- wrong constructor;
- wrong vector constructor;
- negative and over-limit vector counts;
- truncated id/vector/body/header cases;
- missing TL padding;
- state/id cardinality mismatch;
- valid body plus trailing bytes.

## 9. Acceptance criteria

- AC-1: Existing `parse_msgs_all_info`, `TlReader` and public facade remain the only production wire owners;
  no production behavior delta is made.
- AC-2: Authored-only contracts lock R1-R8 without execution.
- AC-3: Desktop mirrors the canonical authored contract blob and keeps production api/lib unchanged.
- AC-4: Existing parity ledger/P0/source-closure/evidence record fixed-U identities and no-production-delta disposition.
- AC-5: Responsibility is `implemented / validation_status=not-run / accepted=false`.
- AC-6: Arbitrary request history/application, connection scheduling, auth-key lifecycle, gzip and complete
  difference decoding remain explicitly open/fail-closed, and no Stage-A executable validation or CI/status/check query occurs.

## 10. Release / rollback

Stage-A checkpoint branches only. No main integration, deployment, packaging or release occurs.
Rollback is the immediately preceding checkpoint ref.

## 11. Provenance

This specification records behavioral responsibility and source identities only. The Fabushi implementation is
source-neutral Rust and does not copy Telegram Desktop C++ implementation text.

## 12. Stage-A compliance record

Initial spec-first disposition: source review confirms no production delta is required.
Authored contract commit/blob, desktop mirror commit/blob, ledger/evidence commit and final compliance readback
will be recorded here after those writes are durably saved. Executable validation remains `not-run`.
