# Telegram MTProto new-session replay — Specification

Status: active  
Owner: shared Telegram network/session runtime  
Last updated: 2026-11-01  
Related task: Fabushi migration task 7457eb1c-11a3-4626-8563-a3e495872536 round 18

## Context

Fixed upstream `telegramdesktop/tdesktop@f985723b552b30fe1c17d377d57f44e61a15a8c4` handles `new_session_created#9ec20908` by adopting the new server salt and resending still-pending requests whose message ids are older than the server-provided `first_msg_id`. The canonical Fabushi `EstablishedSession` currently updates only `server_salt` and continues waiting, so a single outstanding RPC that the server explicitly indicates predates the accepted session can time out instead of being replayed.

## Fixed-U provenance

- `Telegram/SourceFiles/mtproto/session_private.cpp@13cbc76ec723386027b5b4dcac79999b791c4614`
  - `mtpc_new_session_created` reads `first_msg_id`, `unique_id`, and `server_salt`.
  - It updates the session salt.
  - It resends pending requests with message ids less than `first_msg_id`.
  - It forwards the new-session update so higher-level difference recovery can observe it.
- `Telegram/SourceFiles/mtproto/scheme/mtproto.tl@76225965bfc794388db241f3f760db8ab79e84f8`
  - `new_session_created#9ec20908 first_msg_id:long unique_id:long server_salt:long = NewSession`.

## Requirements

- R1: The existing `EstablishedSession` SHALL parse `first_msg_id` and `server_salt` from `new_session_created`.
- R2: The existing server salt SHALL be replaced immediately.
- R3: For `invoke_raw`, if the current outstanding request message id is less than `first_msg_id`, that same logical RPC SHALL be retried once with a fresh message id on the same auth key, transport, and runtime owner.
- R4: If the current request message id is equal to or newer than `first_msg_id`, the session SHALL continue waiting for its result without replay.
- R5: New-session replay SHALL be bounded to one retry per `invoke_raw` call; a repeated replay signal SHALL fail closed rather than loop.
- R6: Container-nested `new_session_created` SHALL apply the same comparison and bounded replay contract.
- R7: Ping/service handling SHALL continue to adopt the new server salt but SHALL NOT invent RPC replay when no content-related RPC is owned by that call.
- R8: No second request queue, runtime, session store, account, provider, or transport owner SHALL be introduced.
- R9: Stage-A contracts SHALL be authored but not executed.
- R10: Higher-level update/difference notification semantics remain a separate responsibility; this slice does not claim that responsibility complete.

## Architecture

This is a refinement of the existing synchronous `telegram-network::EstablishedSession`. Fabushi currently has one outstanding RPC inside `invoke_raw`, so fixed-U pending-request replay maps to that request only. The network owner does not import Telegram Desktop's C++ request queues.

## Failure contract

Malformed `new_session_created`, repeated replay beyond the single retry budget, replay/parity violations, or transport/crypto failure fail closed through existing network errors plus a typed replay-limit error.

## Verification strategy

Author inline Stage-A contracts for the `request_message_id < first_msg_id` predicate and bounded outcome classification. Do not run them until Stage B.

## Acceptance

- canonical platform-core owner implements R1-R9;
- exact implementation is synchronized to the desktop provider path dependency;
- ledger/P0/source-closure record `implemented / not-run / accepted=false`;
- no global unknown/unread decrement is claimed from this responsibility slice.
