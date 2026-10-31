# Telegram MTProto bad_msg_notification exact wire validation

Status: Stage A implementation contract; executable validation deferred.

## Scope

This spec closes only the fixed-U receive-side wire-validation responsibility for `bad_msg_notification#a7eff811` inside the existing canonical `telegram-protocol` / `telegram-network` owners. It does not add a second Telegram runtime, a historical sent-request store, container bookkeeping, gzip handling, or updates difference decoding.

## Fixed-U provenance

- Upstream: `telegramdesktop/tdesktop@f985723b552b30fe1c17d377d57f44e61a15a8c4`.
- `Telegram/SourceFiles/mtproto/scheme/mtproto.tl@76225965bfc794388db241f3f760db8ab79e84f8`
  - `bad_msg_notification#a7eff811 bad_msg_id:long bad_msg_seqno:int error_code:int = BadMsgNotification`.
- `Telegram/SourceFiles/mtproto/session_private.cpp@13cbc76ec723386027b5b4dcac79999b791c4614`
  - the `mtpc_bad_msg_notification` branch first decodes a generated TL object, then reads `vbad_msg_id`, `vbad_msg_seqno`, and `verror_code`.

## Pre-implementation gap

At the live checkpoint before this spec, `native/telegram-network/src/lib.rs` owns a local `BAD_MSG_NOTIFICATION_CONSTRUCTOR` and has two receive branches:
- `EstablishedSession::scan_rpc_body`;
- `EstablishedSession::process_service_body`.

Both branches read `bad_msg_id` from raw offset 4 and `error_code` from raw offset 16. They do not type `bad_msg_seqno`, and the branch logic does not reject trailing bytes before retry/session-reset behavior. The `direct_bad_message_time_recovery` guard also recognizes the constructor/error code through raw offsets before relaxing the incoming message-id time window.

## Requirements

- R1: The canonical protocol owner SHALL export `BAD_MSG_NOTIFICATION_CONSTRUCTOR`, typed `BadMsgNotification`, and `parse_bad_msg_notification(&[u8])`.
- R2: The parser SHALL decode exactly `bad_msg_id:long`, `bad_msg_seqno:int`, and `error_code:int`.
- R3: Wrong constructor, truncation at any field boundary or within any field, and any trailing bytes SHALL fail closed.
- R4: `EstablishedSession::scan_rpc_body` SHALL invoke the typed parser before request correlation, clock correction, session reset, retry signaling, or failure settlement.
- R5: `EstablishedSession::process_service_body` SHALL invoke the same typed parser before request correlation, clock correction, session reset, retry signaling, or failure settlement.
- R6: `direct_bad_message_time_recovery` SHALL only relax the message-id time window for a fully valid typed `bad_msg_notification` whose `error_code` is one of the already-supported direct time-recovery codes.
- R7: A valid notification SHALL affect only the one currently outstanding request when `bad_msg_id` exactly matches that request id. A valid unrelated id SHALL preserve the existing `BadServerMessage` behavior.
- R8: `bad_msg_seqno` SHALL be retained in the typed protocol value without inventing `wasSent/haveSent/_resendingIds/toSend/_sentContainers` history.
- R9: Existing bounded recovery mapping remains unchanged: 16/17/64 -> direct time correction/retry; 32/33 -> session reset/retry; all other codes -> existing bad-server-message failure.
- R10: Desktop provider copies SHALL be synchronized byte-for-byte only after the canonical implementation is durably committed.

## Failure contract

- Wrong constructor, truncated field, or trailing bytes -> protocol parse failure -> `NetworkError::InvalidServiceMessage` before retry/session-reset mutation.
- Malformed body SHALL NOT receive the direct bad-message time-window exception.
- Valid unrelated `bad_msg_id` -> existing `NetworkError::BadServerMessage`.
- Valid matching notification -> preserve the current bounded recovery action for the parsed error code.
- The parsed sequence number does not create historical request/container reconciliation in this slice.

## Verification strategy

Stage A only authors source-level contracts for:
- exact three-field decoding;
- wrong-constructor, truncation, and trailing-byte rejection;
- both existing receive paths consuming the same typed parser;
- malformed objects not relaxing the message-id clock window;
- current-request-only correlation;
- preservation of the bounded recovery-code mapping.

Do not run build, typecheck, lint, schema validation, generators, unit/contract/integration/E2E tests, packaging, deployment, installation, GitHub Actions, checks, or statuses during Stage A.

## Acceptance criteria

- AC1: Complete fixed-U `bad_msg_notification` wire decoding is owned by `telegram-protocol`.
- AC2: Both network receive branches use typed parsing before recovery behavior.
- AC3: `bad_msg_seqno` is decoded and retained; trailing bytes fail closed.
- AC4: The direct clock-window relaxation only accepts a fully valid typed notification.
- AC5: Current-request-only recovery semantics remain bounded to the one outstanding request.
- AC6: Desktop provider copies are byte-identical to canonical protocol/network sources.
- AC7: No gzip, complete difference decoder, or historical multi-request/container scope is claimed.
- AC8: Authored tests remain not-run and acceptance remains false until Stage B executable evidence exists.

## Source-level implementation review

Canonical implementation commit: `0e569130a80a53b48b74f8e27a96004363b30e7e`.
Canonical blobs: protocol api `15e34bbb0e9c5a5d4fdd8b7a9cdedf3d6b123080`, protocol lib `2c4897cfdba4502d8ca880d7a58cfd0e88877bf7`, network `1fc1b1f69e3e37377400d078f916599e0c7b20bc`.
Desktop synchronization commit: `1205f8709995aa4f5fd5dafdc9fb5121fbfce06c`; desktop provider blobs are byte-identical to those canonical api/lib/network blobs. Desktop bookkeeping/evidence commit: `878c2d671d00f5d50dc149647d038913a53ea3ea`.

- R1-R3: satisfied by source inspection — `telegram-protocol` exports the constructor, typed `BadMsgNotification`, and `parse_bad_msg_notification`; the parser consumes all three fixed-U fields and rejects wrong constructor, truncation, and trailing bytes.
- R4-R5: satisfied by source inspection — both existing network receive branches call the typed parser before current-request correlation or recovery mutation.
- R6: satisfied by source inspection — `direct_bad_message_time_recovery` now uses the same typed parser and only accepts the existing 16/17/64 direct-time recovery codes; malformed/trailing bodies return false.
- R7-R9: satisfied by source inspection — current-request correlation remains one-outstanding-request only; `bad_msg_seqno` is retained in the typed value; the existing 16/17/64 and 32/33 bounded recovery mapping is unchanged.
- R10: satisfied by remote blob readback — desktop provider copies are byte-identical to canonical sources.

Acceptance criteria AC1-AC7 are satisfied at source/implementation level. AC8 remains intentionally **blocked/not-run** until Stage B executable validation. No build, typecheck, lint, schema validator, generator, unit/contract/integration/E2E, package, deployment, installation, GitHub Actions, checks, or statuses were run or queried in this review.


## Round 88 adjacent revalidation / no-production-delta (task round 57, 2026-11-01)

Live checkpoint readback before this write is `329c3e89bb5dcf5dbd6d27153fd160d891adbe2f`; fixed U remains
`telegramdesktop/tdesktop@f985723b552b30fe1c17d377d57f44e61a15a8c4`. The adjacent wire-only
responsibility is already exact in the existing canonical owner:

- `native/telegram-protocol/src/api.rs@ebfb3670b5cd097b9ce288782c7954f5cfe9fc43` keeps
  `BAD_MSG_NOTIFICATION_CONSTRUCTOR = 0xa7eff811`,
  `BadMsgNotification { bad_message_id: i64, bad_message_sequence_number: i32, error_code: i32 }`, and
  `parse_bad_msg_notification`, which reads exactly i64/i32/i32 and rejects trailing bytes.
- `native/telegram-protocol/src/tl.rs@89256ab35184b6dc06a095ab34abb9f44e95a264` implements signed little-endian
  `read_i64` / `read_i32` through fixed-size `take_array`, so the field widths are exactly 8/4/4 and
  truncation fails closed.
- `native/telegram-protocol/src/lib.rs@abab39e42f100972484191f0e52e9edd9b88c6cf` exports the same
  parser/type/constructor.
- `native/telegram-network/src/lib.rs@1fc1b1f69e3e37377400d078f916599e0c7b20bc` already consumes that parser in
  both receive branches and in the direct bad-message clock guard. Those recovery/application consumers are
  pre-existing and are not modified by this adjacent wire-only revalidation.

The exact TL object is 20 bytes: 4-byte constructor + 8-byte signed `bad_msg_id` + 4-byte signed
`bad_msg_seqno` + 4-byte signed `error_code`. Therefore the current disposition is **no production delta**.
This round only adds an external authored-only regression contract for exact 20-byte layout, signed extremes,
constructor/field truncation boundaries, wrong constructor, trailing-byte rejection, and exact EOF, then mirrors the
same contract into the desktop provider.

This round deliberately keeps wire parsing separate from bad-time/time-correction recovery, current-request
correlation, retry/session reset, sent-request/container history, acknowledgement/resend application, connection
scheduling, auth-key lifecycle, gzip, and complete difference/differenceSlice decoding/application. Existing
recovery code is evidence that the parser ships, not part of the isolated parser responsibility claimed here.

No build, typecheck, lint, schema validator, generator, unit/contract/integration/E2E test, package, deployment,
installation, GitHub Actions, checks, statuses, or runtime acceptance is executed or queried in Stage A. Executable
validation remains `not-run` and acceptance remains false.


## Round 88 final source-level compliance

Final source-level readback after the durable Stage-A writes confirms:

- live platform-core pre-compliance HEAD: `bc5f9410a520e5e35d40a1c5a1e6a8f2031d6030`;
- no-production-delta spec commit/blob:
  `9984ca1b13185693961ff0735156e55ab035e62a@7155c589c1e4090a534060e42fc9ebce1f8c404d`;
- canonical authored-only contract commit/blob:
  `bc5f9410a520e5e35d40a1c5a1e6a8f2031d6030@184e5bfa7832e9bc7c7d4f594011b032382e11d3`;
- canonical production api/tl/lib/network blobs remain unchanged at
  `ebfb3670b5cd097b9ce288782c7954f5cfe9fc43` /
  `89256ab35184b6dc06a095ab34abb9f44e95a264` /
  `abab39e42f100972484191f0e52e9edd9b88c6cf` /
  `1fc1b1f69e3e37377400d078f916599e0c7b20bc`;
- desktop contract mirror commit `58237abd9bcb64671d1bbfe88a3cdc8bdca5a2a4` is byte-identical at
  `184e5bfa7832e9bc7c7d4f594011b032382e11d3`;
- desktop bookkeeping/evidence commit `619e9cabb4353a268f7b74cbfa58116c2e7bc907` records the same
  `TDRP-R9-MTPROTO-BAD-MSG-NOTIFICATION-WIRE-001` row at
  `implemented / validation_status=not-run / accepted=false`, with ledger blob
  `0867d2348599d6dcd2542b6045babcbe466c877d` and unchanged coverage
  `97 responsibilities / 16172 unknown / 16145 unread / 0 omitted`.

For the isolated adjacent parser responsibility, constructor `0xa7eff811`, signed 8/4/4 decoding, truncation
fail-closed behavior and strict EOF are source-level satisfied with **no production delta**. The existing
telegram-network consumers prove shipping reachability but their bad-time/time-correction recovery,
current-request correlation, retry/session reset, sent/container history and other application semantics remain
outside the wire-only responsibility claimed in this round.

The newly authored external contracts cover exact 20-byte layout, signed i64/i32/i32 extremes, wrong constructor,
constructor/field truncation boundaries, trailing bytes and exact EOF. They remain intentionally not run.
No build, typecheck, lint, schema validator, generator, unit/contract/integration/E2E test, package, deployment,
installation, GitHub Actions, checks, statuses, or runtime acceptance was executed or queried. Stage-B executable
evidence and independent release ACCEPT remain open.
