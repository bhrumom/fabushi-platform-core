# Telegram MTProto msg_new_detailed_info wire responsibility — Specification

Status: active  
Owner: Fabushi shared telegram-protocol  
Last updated: 2026-11-01  
Related issue/task/PR: TDRP-001 / task 7457eb1c-11a3-4626-8563-a3e495872536 / round 56

## 1. Context / problem

Fixed upstream U remains `telegramdesktop/tdesktop@f985723b552b30fe1c17d377d57f44e61a15a8c4`.
Its MTProto schema declares
`msg_new_detailed_info#809db6df answer_msg_id:long bytes:int status:int = MsgDetailedInfo`.
Shipping `SessionPrivate::handle` contains a distinct `mtpc_msg_new_detailed_info` branch. The independent wire
responsibility must be closed without importing bad-time handling, received-message history, ack/resend application,
or broader session lifecycle into the shared parser.

## 2. Goal

Close only the exact fixed-U typed wire responsibility for `msg_new_detailed_info#809db6df` in the existing
source-neutral `telegram-protocol` owner, with exact 8/4/4 field widths, truncation failure, and strict trailing
validation. Record a no-production-delta disposition if the existing parser already satisfies the contract.

## 3. Non-goals / out of scope

This slice does not implement or claim parity for:
- bad-time handling or the upstream decision to ignore this service message when `info.badTime` is set;
- `_receivedMessageIds` history;
- acknowledgement or resend enqueueing/application;
- arbitrary request history, connection scheduling/racing/send retry;
- temporary/permanent auth-key binding, expiry, persistence, or teardown;
- gzip handling or complete difference/differenceSlice decoding/application;
- Stage-B executable verification or release acceptance.

## 4. Requirements

- R1: Constructor must be exactly `0x809db6df`.
- R2: `answer_msg_id` must consume exactly eight bytes as signed little-endian TL `long` / Rust `i64`.
- R3: `bytes` and `status` must each consume exactly four bytes as signed little-endian TL `int` / Rust `i32`.
- R4: Truncation in the constructor or any field must fail closed.
- R5: A valid object must end at exact EOF; any trailing byte must fail closed.
- R6: Parsing must be side-effect free and must not inspect or mutate bad-time state, received-message history,
  acknowledgement queues, resend queues, request history, or connection/session scheduling.
- R7: Stage A may write durable spec, contracts, fixtures/oracles and bookkeeping only; contracts are not executed
  and no Actions/status/check query, build, typecheck, lint, validator, generator, package, deploy, install, or
  runtime acceptance is performed.

## 5. Current state

Remote source readback shows the canonical owner already contains:
- `native/telegram-protocol/src/api.rs@ebfb3670b5cd097b9ce288782c7954f5cfe9fc43`
  - `MSG_NEW_DETAILED_INFO_CONSTRUCTOR = 0x809db6df`;
  - `MsgNewDetailedInfo { answer_message_id: i64, bytes: i32, status: i32 }`;
  - `parse_msg_new_detailed_info`, which reads exactly i64/i32/i32 and rejects trailing bytes.
- `native/telegram-protocol/src/tl.rs@89256ab35184b6dc06a095ab34abb9f44e95a264`
  - `TlReader::read_i64` and `read_i32` use exact little-endian fixed-width reads;
  - bounded `take` / `take_array` fail closed on truncation.
- `native/telegram-protocol/src/lib.rs@abab39e42f100972484191f0e52e9edd9b88c6cf`
  - exports the same parser/type/constructor.

The exact encoded object is 20 bytes: 4 constructor + 8 + 4 + 4.

## 6. Target state

No production behavior delta is required. The existing parser remains the sole production wire owner. Stage A adds
authored-only regression contracts and the existing migration bookkeeping surfaces record this responsibility as
`implemented / validation_status=not-run / accepted=false`.

## 7. Architecture and ownership boundaries

Existing owner candidates:
- canonical shared `native/telegram-protocol`;
- desktop tracked `source/mahayana/mahayana-rs/providers/telegram-protocol` mirror.

Rejected owners:
- network/session history owner: application/history behavior is outside the wire-only boundary;
- a second Telegram service-message parser: duplicate owner;
- a desktop-only divergent parser: violates canonical shared ownership.

No new owner or ADR is required.

## 8. Interfaces / contracts / schemas / data flow

Wire contract:
`0x809db6df + i64 answer_msg_id + i32 bytes + i32 status + EOF -> MsgNewDetailedInfo`.

Fixed-U shipping order is semantically important:
`SessionPrivate::handle` checks `info.badTime` and may ignore the message before typed decoding; when not ignored,
`MTPMsgDetailedInfo::read(from,end)` performs the typed decode, and only after successful decoding does the branch
consult `_receivedMessageIds` and enqueue ack or resend work. This specification owns only the typed object
boundary and deliberately does not reproduce the surrounding application decisions.

## 9. Constraints and non-functional requirements

- Existing-owner-first; source-neutral Rust only.
- Preserve full signed TL integer domains; do not impose application-level range meaning on `bytes` or `status`.
- No second provider/runtime/history store.
- Stage A remains strictly zero-CI and zero executable validation.

## 10. Failure modes and edge cases

Expected fail-closed cases:
- truncated constructor;
- wrong constructor;
- truncated `answer_msg_id`;
- truncated `bytes`;
- truncated `status`;
- a valid 20-byte object followed by any trailing input.

Parser closure must not be used to claim bad-time/history/resend/ack/session parity.

## 11. Implementation strategy

1. Keep production `api.rs/tl.rs/lib.rs` unchanged because exact behavior already exists.
2. Author canonical contracts that lock constructor, 8/4/4 widths, signedness, little-endian order, truncation and EOF.
3. Mirror the exact authored contract into the desktop tracked provider.
4. Update the existing parity ledger, P0/source-closure and round evidence; do not create a second progress system.
5. Perform source-level compliance readback only; executable verification remains deferred to Stage B.

## 12. Verification / test strategy

Authored-only contracts, not run in Stage A:
- exact 20-byte layout using `i64::MIN`, `i32::MIN`, and `i32::MAX`;
- wrong constructor;
- truncation before constructor completion and within each 8/4/4 field;
- valid object plus trailing byte;
- exact EOF via `TlReader`.

## 13. Acceptance criteria / Definition of Done

- AC-1: Existing `parse_msg_new_detailed_info`, `TlReader`, and public facade remain the only production wire owners;
  no production behavior delta is made.
- AC-2: Authored-only contracts lock R1-R5 without execution.
- AC-3: Desktop mirrors the canonical authored contract blob and production `api.rs`/`lib.rs` remain unchanged.
- AC-4: Existing parity ledger/P0/source-closure/evidence record fixed-U identities, no-production-delta disposition,
  exact 8/4/4 widths, strict EOF, and separation from bad-time/history/ack-resend application semantics.
- AC-5: Responsibility is `implemented / validation_status=not-run / accepted=false`.
- AC-6: Broader session/history/auth-key/gzip/difference responsibilities remain explicitly open/fail-closed.

## 14. Release / migration / rollback

Stage-A checkpoint branches only. No main integration, deployment, packaging or release occurs. Rollback is the
immediately preceding checkpoint ref.

## 15. Observability / evidence

Source identities, authored contract blob identities, checkpoint commits, ledger row, P0/source-closure entry and
round evidence file are the Stage-A evidence. Runtime evidence is intentionally absent until Stage B.

## 16. References / provenance

- `telegramdesktop/tdesktop@f985723b552b30fe1c17d377d57f44e61a15a8c4`
- `Telegram/SourceFiles/mtproto/scheme/mtproto.tl@76225965bfc794388db241f3f760db8ab79e84f8`
- `Telegram/SourceFiles/mtproto/session_private.cpp@13cbc76ec723386027b5b4dcac79999b791c4614`

This specification records behavioral responsibility and source identities only. The Fabushi implementation is
source-neutral Rust and copies no Telegram Desktop C++ implementation text.

## 17. Spec compliance record

| Requirement / AC | Status | Evidence / reason |
| --- | --- | --- |
| R1-R5 | passed | Remote source readback confirms `api.rs@ebfb3670b5cd097b9ce288782c7954f5cfe9fc43` preserves constructor 0x809db6df, signed i64/i32/i32 field order and exact EOF; `tl.rs@89256ab35184b6dc06a095ab34abb9f44e95a264` preserves exact 8-byte i64 / 4-byte i32 little-endian reads and fail-closed truncation; `lib.rs@abab39e42f100972484191f0e52e9edd9b88c6cf` exports the same parser/type/constructor. |
| R6 | passed | Fixed-U `SessionPrivate::handle` keeps bad-time gating outside and before the typed reader; for non-bad-time messages `MTPMsgDetailedInfo::read(from,end)` succeeds before `_receivedMessageIds` lookup or ack/resend enqueueing. Fabushi parser is side-effect free and owns none of those session semantics. |
| R7 | passed | Stage A performed source/blob readback and authored/saved code/docs only. No build, typecheck, lint, validator, generator, unit/contract/integration/E2E execution, Actions/status/check query, package, deploy, install or runtime acceptance was performed. |
| AC-1 | passed | Production api/tl/lib blobs remain ebfb3670b5cd097b9ce288782c7954f5cfe9fc43 / 89256ab35184b6dc06a095ab34abb9f44e95a264 / abab39e42f100972484191f0e52e9edd9b88c6cf; no production behavior delta was made. |
| AC-2 | passed | Canonical authored-only contract commit `9154bcf48a0d27efe2fa459a9670100c99d29e25` / blob `feedf5e25c739e0deca7a26f356a25a9d1a9f46b` covers exact 20-byte layout, signed i64/i32/i32 extremes, wrong constructor, truncation at constructor/each field boundary, trailing bytes and exact EOF. It was not executed. |
| AC-3 | passed | Desktop mirror commit `efeb7cef34a4582deb0a3631856ded581889f6bd` has byte-identical contract blob `feedf5e25c739e0deca7a26f356a25a9d1a9f46b`; desktop production api/lib remain ebfb3670b5cd097b9ce288782c7954f5cfe9fc43 / abab39e42f100972484191f0e52e9edd9b88c6cf. |
| AC-4 | passed | Desktop ledger/evidence/P0/source-closure record fixed-U identities, no-production-delta disposition, exact 8/4/4 widths, strict EOF, and separation from bad-time/history/ack-resend application semantics. |
| AC-5 | passed | Existing parity ledger records `TDRP-R9-MTPROTO-MSG-NEW-DETAILED-INFO-WIRE-001` as `implemented / validation_status=not-run / accepted=false`; coverage is responsibilities_total=97, unknown=16172, unread=16145, omitted=0. |
| AC-6 | passed | Bad-time handling, `_receivedMessageIds`, ack/resend application, arbitrary request history, connection scheduling/racing/send retry, auth-key binding/expiry/persistence/teardown, gzip and complete difference/differenceSlice decoding/application remain explicitly open/fail-closed. |

Source-level compliance is complete for this isolated `msg_new_detailed_info` wire slice. This is not executable
verification, candidate readiness, release acceptance, session/history/application closure, broader source/legal
closure, or independent release ACCEPT.
