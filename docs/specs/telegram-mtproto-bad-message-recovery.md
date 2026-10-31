# Telegram MTProto bad-message recovery — Specification

Status: active  
Owner: shared Telegram network/session runtime  
Last updated: 2026-11-01  
Related task: Fabushi migration task 7457eb1c-11a3-4626-8563-a3e495872536 round 18

## 1. Context / problem

Fixed upstream `telegramdesktop/tdesktop@f985723b552b30fe1c17d377d57f44e61a15a8c4` treats selected `bad_msg_notification` codes as recoverable session lifecycle signals. The canonical Fabushi `telegram-network` owner currently parses `bad_server_salt` but converts every `bad_msg_notification` into a terminal `BadServerMessage`, so clock-skew and sequence/session-recovery responsibilities are not yet represented in the production network owner.

## 2. Goal

Implement bounded source-neutral recovery for the fixed-U recoverable bad-message classes in the existing `EstablishedSession`, without creating a second transport, account, runtime, or persistence owner.

## 3. Non-goals

- No Telegram C++ runtime, TDLib, sidecar, subprocess, or provider duplication.
- No unbounded retry loop.
- No silent recovery for unknown/fatal bad-message codes.
- No new message/domain state store.
- No Stage-A build, lint, generator, test, package, deployment, installation, workflow/status query, or runtime acceptance.

## 4. Fixed-U provenance

- `Telegram/SourceFiles/mtproto/session_private.cpp@13cbc76ec723386027b5b4dcac79999b791c4614`
  - codes 16/17: correct server time and resend;
  - codes 32/33: reset the session because message-id/sequence state is inconsistent;
  - code 64: resend a rejected container/message;
  - other codes remain fatal/protocol-error behavior.
- `Telegram/SourceFiles/mtproto/scheme/mtproto.tl@76225965bfc794388db241f3f760db8ab79e84f8`
  - `bad_msg_notification#a7eff811`;
  - `bad_server_salt#edab447b`;
  - `new_session_created#9ec20908`;
  - `msgs_ack#62d6b459`.

## 5. Requirements

- R1: `telegram-network::EstablishedSession` SHALL recognize only recoverable bad-message codes 16, 17, 32, 33, and 64.
- R2: Codes 16/17 SHALL derive the current server time from the authenticated server message id, update the session clock offset, and retry the same RPC at most once with a fresh client message id.
- R3: Codes 32/33 SHALL reset the session id and content-related sequence counter, preserve the existing auth key/server salt/transport, and retry the same RPC at most once.
- R4: Code 64 SHALL perform one bounded fresh-message retry; it SHALL NOT create an unbounded container replay loop.
- R5: Unknown/fatal codes SHALL remain `NetworkError::BadServerMessage`.
- R6: A recoverable bad-message notification SHALL apply only when its `bad_msg_id` equals the current request message id; unrelated notifications fail closed.
- R7: Server-message replay/parity protection SHALL remain active. A direct authenticated bad-message notification may use its own server message time as the narrow reference when local-clock validation would otherwise reject it as outside the -300/+30 second window.
- R8: Outgoing encrypted RPC and ping message ids SHALL use the current in-session clock offset after a time correction.
- R9: Existing `bad_server_salt`, acknowledgement, auth-key, encryption, and one-session ownership semantics SHALL remain intact.
- R10: Stage-A tests SHALL be authored but not executed.

## 6. Architecture / ownership

The existing `native/telegram-network` crate remains the unique owner for socket/session retry policy. `telegram-protocol` continues to own message-id generation/validation primitives and cryptography; `telegram-runtime` continues to own the single client lifecycle. No new canonical root is introduced.

## 7. Failure and recovery contract

Recovery is bounded to one retry per `invoke_raw`/ping call. A second recoverable rejection in the same call returns `BadServerMessage`. Failure to produce a new session id, invalid/malformed service messages, replayed server messages, mismatched bad_msg_id, or fatal codes fail closed.

## 8. Verification strategy

Stage A authors focused unit/contract cases for classification, server-time derivation, bounded retry state transitions, and session-reset state. Tests are not executed until the consolidated Stage-B campaign.

## 9. Acceptance criteria

- AC-1: The canonical shared network owner performs the recovery transitions in R1-R9.
- AC-2: Desktop shipping provider source is synchronized to the same network implementation.
- AC-3: Authored tests/contracts exist with no Stage-A execution evidence.
- AC-4: TDRP ledger/P0/source-closure records remain `implemented / validation_status=not-run / accepted=false`.

## 10. Release / rollback

Stage-A checkpoint refs only. Rollback is the preceding checkpoint commit. No release or main integration is performed in this round.
