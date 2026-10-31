# Telegram MTProto detailed-answer resend — Specification

Status: active  
Owner: shared Telegram protocol/network runtime  
Last updated: 2026-11-01  
Related task: Fabushi migration task 7457eb1c-11a3-4626-8563-a3e495872536 round 18

## Context

Fixed upstream `telegramdesktop/tdesktop@f985723b552b30fe1c17d377d57f44e61a15a8c4` handles `msg_detailed_info#276d3ec6` by acknowledging the referenced sent request and requesting retransmission of `answer_msg_id` when that answer has not been received. Fabushi's canonical `EstablishedSession::invoke_raw` has one outstanding content RPC and currently ignores this service message, so a server-known answer that was lost can become an RPC timeout.

## Fixed-U provenance

- `Telegram/SourceFiles/mtproto/session_private.cpp@13cbc76ec723386027b5b4dcac79999b791c4614`
  - `mtpc_msg_detailed_info` reads `msg_id` and `answer_msg_id`;
  - the sent request is considered acknowledged;
  - a missing answer is queued for `msg_resend_req`.
- `Telegram/SourceFiles/mtproto/scheme/mtproto.tl@76225965bfc794388db241f3f760db8ab79e84f8`
  - `msg_detailed_info#276d3ec6 msg_id:long answer_msg_id:long bytes:int status:int`;
  - `msg_resend_req#7d861a08 msg_ids:Vector<long>`.

## Requirements

- R1: The canonical protocol owner SHALL expose a typed `build_msg_resend_req(&[i64])` using the fixed-U constructor.
- R2: `EstablishedSession::invoke_raw` SHALL recognize `msg_detailed_info` only when its `msg_id` equals the current outstanding request.
- R3: For that matching request, the same session SHALL send one non-content-related `msg_resend_req` containing exactly `answer_msg_id`, then continue waiting for the RPC result.
- R4: An unrelated `msg_detailed_info.msg_id` SHALL be ignored for the current call and SHALL NOT mutate retry state.
- R5: The resend service request SHALL use the same auth key, transport, message-id guard, session id and sequence owner.
- R6: This slice SHALL NOT create a sent-request history or multi-request queue. `msg_new_detailed_info`, `msgs_state_req/info/all_info` and full multi-request state reconciliation remain separate responsibilities.
- R7: Existing replay/parity/time validation and acknowledgement behavior SHALL remain in force.
- R8: Stage-A tests SHALL be authored but not executed.

## Architecture

The protocol builder belongs beside `build_msgs_ack`. The network owner consumes it through the existing `send_encrypted_message(..., false)` service-message path. The product/runtime/renderer layers remain unchanged.

## Verification strategy

Author protocol encoding and network predicate contracts. Do not execute them until Stage B.

## Acceptance

- platform-core shared owner implements R1-R8;
- exact sources are synchronized into the desktop provider chain;
- ledger/P0/source-closure remain `implemented / not-run / accepted=false`;
- global unknown/unread counts are unchanged because this is a responsibility slice.
