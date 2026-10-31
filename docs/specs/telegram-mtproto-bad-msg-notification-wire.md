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
