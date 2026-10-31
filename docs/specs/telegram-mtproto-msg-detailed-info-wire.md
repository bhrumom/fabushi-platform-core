# Telegram MTProto msg_detailed_info wire responsibility

Status: Stage A implementation bookkeeping; executable validation intentionally deferred.
Task: TDRP-001 / task 7457eb1c-11a3-4626-8563-a3e495872536 / round 55.

## 1. Purpose

Close only the fixed-U service-message wire responsibility for
`msg_detailed_info#276d3ec6 msg_id:long answer_msg_id:long bytes:int status:int = MsgDetailedInfo`
inside the existing source-neutral `telegram-protocol` owner.

This slice is deliberately separate from Telegram Desktop request acknowledgement, received-message history,
resend/application semantics, connection scheduling, and other session lifecycle behavior.

## 2. Fixed source authority

Fixed upstream U remains:
`telegramdesktop/tdesktop@f985723b552b30fe1c17d377d57f44e61a15a8c4`.

Relevant fixed-U identities:
- `Telegram/SourceFiles/mtproto/scheme/mtproto.tl@76225965bfc794388db241f3f760db8ab79e84f8`
- `Telegram/SourceFiles/mtproto/session_private.cpp@13cbc76ec723386027b5b4dcac79999b791c4614`

The schema declares:
`msg_detailed_info#276d3ec6 msg_id:long answer_msg_id:long bytes:int status:int = MsgDetailedInfo`.

Shipping `SessionPrivate::handle` first calls `MTPMsgDetailedInfo::read(from, end)` and obtains typed
`msg_id`, `answer_msg_id`, `bytes`, and `status`. Only after that exact typed read succeeds does it enter
application behavior: optional bad-time recovery, `requestsAcked`, lookup in `_receivedMessageIds`, and enqueueing
of acknowledgement or resend requests for `answer_msg_id`. Those request-history/application effects are outside
this wire-only slice.

## 3. Existing owner disposition

No production behavior delta is required.

Current canonical production owner:
- `native/telegram-protocol/src/api.rs@ebfb3670b5cd097b9ce288782c7954f5cfe9fc43`
  - `MSG_DETAILED_INFO_CONSTRUCTOR = 0x276d3ec6`
  - `MsgDetailedInfo { message_id: i64, answer_message_id: i64, bytes: i32, status: i32 }`
  - `parse_msg_detailed_info`
- `native/telegram-protocol/src/tl.rs@89256ab35184b6dc06a095ab34abb9f44e95a264`
  - `TlReader::read_i64` reads exactly eight little-endian bytes as signed `i64`
  - `TlReader::read_i32` reads exactly four little-endian bytes as signed `i32`
  - bounded `take` / `take_array` fail closed on truncation
- `native/telegram-protocol/src/lib.rs@abab39e42f100972484191f0e52e9edd9b88c6cf`
  - public export of the same parser/type/constructor.

The desktop tracked provider mirror has byte-identical production `api.rs` and `lib.rs` blobs for this slice.

`parse_msg_detailed_info` already:
1. requires constructor `0x276d3ec6`;
2. reads `msg_id` as signed little-endian `i64`;
3. reads `answer_msg_id` as signed little-endian `i64`;
4. reads `bytes` as signed little-endian `i32`;
5. reads `status` as signed little-endian `i32`;
6. fails closed if any field is truncated;
7. requires exact EOF and rejects trailing input;
8. is side-effect free and does not consult or mutate request history, received-message state, acknowledgement queues,
   resend queues, or connection/session scheduling.

The exact encoded object is 28 bytes: 4 constructor + 8 + 8 + 4 + 4. This wire layer intentionally preserves the
full signed TL integer domains and does not impose application-level meaning or range checks on `bytes` or `status`.

## 4. Requirements

- R1: Constructor must be exactly `0x276d3ec6`.
- R2: `msg_id` and `answer_msg_id` must each consume exactly eight bytes and preserve signed TL `long` / Rust `i64`
  little-endian semantics.
- R3: `bytes` and `status` must each consume exactly four bytes and preserve signed TL `int` / Rust `i32`
  little-endian semantics.
- R4: Truncation in the constructor or any of the four fields must fail closed; no short read or width substitution is allowed.
- R5: A valid object must end at exact EOF; any trailing byte must fail closed.
- R6: Parsing must remain side-effect free and must not perform `requestsAcked`, received-message lookup,
  resend/ack enqueueing, bad-time correction, or other application/session history behavior.
- R7: Stage A records a no-production-delta disposition and may author contracts/bookkeeping only; no executable
  validation, GitHub Actions/status/check query, build, package, deploy, install, or runtime acceptance is performed.

## 5. Failure behavior

Expected fail-closed cases include:
- truncated constructor;
- wrong constructor;
- truncated `msg_id`;
- truncated `answer_msg_id`;
- truncated `bytes`;
- truncated `status`;
- a valid 28-byte object followed by any trailing input.

No failure path may silently route into a second Telegram parser/runtime or claim request-history/application parity.

## 6. Existing-owner-first architecture

Existing owner candidates:
- canonical shared `native/telegram-protocol`;
- desktop tracked `source/mahayana/mahayana-rs/providers/telegram-protocol` mirror.

Rejected owners:
- `telegram-network` / session request-history owner for this pure wire parse;
- a second Telegram service-message parser;
- a desktop-only parser that diverges from the canonical shared protocol owner.

No new owner or ADR is required.

## 7. Production composition

The production composition remains:
existing account/session lifecycle -> embedded MTProto transport -> existing shared telegram-protocol wire parser.

This slice adds no provider, sidecar, subprocess, second request-history store, second message store, or UI surface.

## 8. Authored-only contract plan

Stage A contracts must be written but not executed. They must cover:
- an exact 28-byte object using `i64::MIN`, `i64::MAX`, `i32::MIN`, and `i32::MAX` to lock field order,
  signedness, widths, and little-endian encoding;
- wrong constructor;
- truncation before the constructor completes and inside each 8/8/4/4-byte field;
- a valid object plus trailing input.

## 9. Acceptance criteria

- AC-1: Existing `parse_msg_detailed_info`, `TlReader`, and the public facade remain the only production wire owners;
  no production behavior delta is made.
- AC-2: Authored-only contracts lock R1-R5 without execution.
- AC-3: Desktop mirrors the canonical authored contract blob and keeps production `api.rs`/`lib.rs` unchanged.
- AC-4: Existing parity ledger/P0/source-closure/evidence record fixed-U identities, no-production-delta disposition,
  exact 8/8/4/4 field widths, strict EOF, and the application/history separation.
- AC-5: Responsibility is `implemented / validation_status=not-run / accepted=false`.
- AC-6: `requestsAcked`, `_receivedMessageIds`, resend/ack application, bad-time correction, arbitrary request history,
  connection scheduling, auth-key lifecycle, gzip, and complete difference decoding remain explicitly open/fail-closed.

## 10. Release / rollback

Stage-A checkpoint branches only. No main integration, deployment, packaging, or release occurs.
Rollback is the immediately preceding checkpoint ref.

## 11. Provenance

This specification records behavioral responsibility and source identities only. The Fabushi implementation is
source-neutral Rust and does not copy Telegram Desktop C++ implementation text.

## 12. Stage-A compliance record

Initial spec-first disposition: source review confirms no production delta is required.
Authored contract commit/blob, desktop mirror commit/blob, ledger/evidence commit and final compliance readback
will be recorded here after those writes are durably saved. Executable validation remains `not-run`.
