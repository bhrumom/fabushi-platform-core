# Telegram MTProto single-RPC state reconciliation — Specification

Status: active  
Owner: shared Telegram protocol/network runtime  
Last updated: 2026-11-01  
Related task: Fabushi migration task 7457eb1c-11a3-4626-8563-a3e495872536 round 19

## Context

Fixed upstream `telegramdesktop/tdesktop@f985723b552b30fe1c17d377d57f44e61a15a8c4` uses `msg_new_detailed_info`, `msgs_state_req`, `msgs_state_info`, `msgs_all_info`, and `SessionPrivate::handleMsgsStates` to recover from uncertain delivery and missing answer messages. Telegram Desktop keeps a multi-request sent/resend history. Fabushi's canonical `EstablishedSession::invoke_raw` intentionally has one outstanding content RPC and no parallel sent-request queue.

This slice implements only the state that the existing owner can represent exactly: the current RPC plus at most one state query for that RPC.

## Fixed-U provenance

- `Telegram/SourceFiles/mtproto/session_private.cpp@13cbc76ec723386027b5b4dcac79999b791c4614`
  - `mtpc_msgs_state_info` correlates a state response with the service request that asked for it;
  - `mtpc_msgs_all_info` applies state bytes to the referenced message ids;
  - `SessionPrivate::handleMsgsStates` treats `(state & 0x07) == 0x04` as received/acknowledged and otherwise schedules resend;
  - `mtpc_msg_new_detailed_info` acknowledges an already received answer id or requests its retransmission.
- `Telegram/SourceFiles/mtproto/scheme/mtproto.tl@76225965bfc794388db241f3f760db8ab79e84f8`
  - `msgs_state_req#da69fb52 msg_ids:Vector<long>`;
  - `msgs_state_info#04deb57d req_msg_id:long info:string`;
  - `msgs_all_info#8cc0d131 msg_ids:Vector<long> info:string`;
  - `msg_new_detailed_info#809db6df answer_msg_id:long bytes:int status:int`;
  - `msg_resend_req#7d861a08 msg_ids:Vector<long>`.

## Requirements

- R1: The canonical protocol owner SHALL encode fixed-U `msgs_state_req` and parse fixed-U `msgs_state_info` / `msgs_all_info` with bounded TL readers.
- R2: The canonical network owner SHALL issue at most one non-content `msgs_state_req([current_request_message_id])` when the normal current-RPC receive window is exhausted.
- R3: A `msgs_state_info` SHALL affect the current RPC only when `req_msg_id` equals that one state-query message id and the response contains exactly one state byte.
- R4: `msgs_all_info` SHALL affect the current RPC only when its id vector explicitly contains the current request message id.
- R5: For the current request, `(state & 0x07) != 0x04` SHALL request one bounded replay through the existing outer retry loop; `0x04` SHALL keep waiting without replay.
- R6: Repeated state-driven replay after the existing retry budget SHALL fail closed with a typed reconciliation-limit error.
- R7: `msg_new_detailed_info` SHALL use the existing server-message replay window only: an already seen `answer_msg_id` is acknowledged; an unseen `answer_msg_id` is requested with one non-content `msg_resend_req`.
- R8: The existing `msg_detailed_info` path SHALL use the same seen-answer reconciliation rather than redundantly requesting an answer already in the replay window.
- R9: All service messages SHALL use the same auth key, session id, sequence owner, message-id guard and transport.
- R10: No `haveSentMap`, `_resendingIds`, `_stateAndResendRequests`, multi-request request history, background resend queue, second runtime, second provider or second session owner SHALL be introduced.
- R11: State for unrelated request ids, unrelated state-query ids, or unknown multi-request history SHALL be ignored for the current call and remain explicitly open rather than guessed.
- R12: Stage-A contracts SHALL be authored but not executed.

## Architecture

The protocol crate owns TL constructors/builders/parsers. The network crate owns the current `invoke_raw` request id, the optional single state-query id, the replay-window observation, and the bounded retry decision. Desktop consumes the exact canonical crates through the existing provider path chain.

## Deferred multi-request responsibilities

Full Telegram Desktop semantics that require more than the single outstanding RPC remain fail-closed and open, including arbitrary historical request reconciliation, multiple simultaneous state queries, cancellation of already queued resends, container-wide resend bookkeeping, and durable sent-request history.

## Verification strategy

Author protocol encoding/parsing contracts and pure network reconciliation predicate contracts. Do not run build, lint, tests, generators, packaging, deployment or runtime acceptance until Stage B.

## Acceptance

- platform-core shared owner implements R1-R12;
- desktop provider copies are synchronized byte-for-byte with the canonical modified sources;
- ledger/P0/source-closure record the implemented single-RPC slice and the still-open multi-request queue responsibilities;
- validation remains `not-run`, `accepted=false`, and global unknown/unread source-entry counts are unchanged for this responsibility slice.
