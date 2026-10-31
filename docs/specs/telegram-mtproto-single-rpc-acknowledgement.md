# Telegram MTProto single-RPC acknowledgement — Specification

Status: active  
Owner: shared Telegram protocol/network runtime  
Last updated: 2026-11-01  
Related issue/task/PR: Fabushi migration task 7457eb1c-11a3-4626-8563-a3e495872536 round 20

## 1. Context / problem

Fixed upstream `telegramdesktop/tdesktop@f985723b552b30fe1c17d377d57f44e61a15a8c4` handles `mtpc_msgs_ack` in `SessionPrivate::handleOneReceived`. Telegram Desktop parses the `msgs_ack#62d6b459 msg_ids:Vector<long>` service message, optionally uses sent-request timing for clock correction, and passes ids to `requestsAcked`. `requestsAcked` deliberately does not remove a sent request when a normal ACK arrives and that request still has a callback requiring a response.

Fabushi's canonical `EstablishedSession::invoke_raw` has exactly one outstanding content RPC and no Telegram Desktop-style sent/resend history. Before this slice, `msgs_ack` during an RPC fell through the generic unknown-service-message path without validating its TL shape, while the ping service path treated the constructor as an unconditional no-op.

## 2. Goal

Represent exactly the fixed-U acknowledgement semantics that the existing single-outstanding-RPC owner can carry: parse and validate `msgs_ack`, recognize whether it mentions the current RPC, and preserve that callback RPC as outstanding until `rpc_result` or an existing bounded recovery path settles it.

## 3. Non-goals / out of scope

- arbitrary historical `haveSentMap` acknowledgement bookkeeping;
- `_resendingIds` cancellation or `toSendMap` mutation;
- container-ACK fan-out through `_sentContainers`;
- `_stateAndResendRequests` lifecycle;
- fast-request clock correction that depends on per-request send timestamps/history;
- introducing a second sent queue, runtime, provider, transport, session, or message store.

Those responsibilities remain open/fail-closed until a canonical owner can represent them without duplicating state truth.

## 4. Requirements

- R1: The canonical protocol owner SHALL parse fixed-U `msgs_ack#62d6b459 msg_ids:Vector<long>` with the existing bounded TL reader.
- R2: The parser SHALL reject a wrong constructor, malformed vector, truncated ids, or trailing bytes.
- R3: The canonical network RPC path SHALL explicitly route `MSGS_ACK_CONSTRUCTOR` through that parser instead of generic fallthrough.
- R4: An ACK containing the current `request_message_id` SHALL NOT produce `RpcResult`, clear the request, trigger replay, or otherwise settle the callback RPC; the call SHALL continue waiting for `rpc_result` or existing bounded recovery.
- R5: ACK ids unrelated to the current single RPC SHALL have no effect on that RPC.
- R6: The ping/service-message path SHALL also validate `msgs_ack` shape and continue waiting for `pong`; a plain ACK SHALL NOT settle a ping.
- R7: Malformed ACK payloads SHALL fail closed as the existing invalid-service-message error.
- R8: No multi-request sent/resend/history state SHALL be added.
- R9: Stage-A protocol/network contracts SHALL be authored but not executed.

## 5. Current state

The protocol owner can encode `msgs_ack` but cannot parse it. The network owner has the constructor constant; `invoke_raw` has no explicit ACK arm and the ping path accepts the constructor without validating payload shape.

## 6. Target state

Both RPC and ping service-message paths validate `msgs_ack`. The RPC path may recognize the current request id for traceable semantics, but normal ACK receipt remains non-settling for the callback RPC, matching fixed-U `requestsAcked(..., byResponse=false)` behavior.

## 7. Architecture and ownership boundaries

`native/telegram-protocol` owns TL parsing. `native/telegram-network::EstablishedSession` owns the one current RPC/ping and service-message dispatch. Desktop consumes synchronized copies through the existing provider chain. No product/UI owner is added.

## 8. Interfaces / contracts / schemas / data flow

`parse_msgs_ack(&[u8]) -> Result<Vec<i64>, ApiRequestError>` decodes the fixed-U constructor and vector. Network dispatch maps parser failure to `NetworkError::InvalidServiceMessage`. A pure current-request membership predicate documents the only id correlation this owner can represent.

## 9. Constraints and non-functional requirements

The implementation must remain bounded by existing TL/vector limits, preserve current auth/session/sequence/message-id ownership, avoid background queues, and keep Stage A at zero CI/runtime validation.

## 10. Failure modes and edge cases

Wrong constructor, truncated vector entries, malformed vector length, and trailing bytes fail closed. Empty ACK vectors are valid but have no effect. Duplicate ids and unrelated ids have no effect on callback settlement. Multi-request bookkeeping is intentionally not inferred.

## 11. Implementation strategy

Add the protocol parser and parser contracts, export it from the protocol crate, route ACKs explicitly in both network service dispatch paths, and add pure contract coverage proving current-request recognition does not imply settlement.

## 12. Verification / test strategy

Author unit/contract tests only. Do not execute build, lint, unit, contract, integration, packaged, deployment, or runtime checks during Stage A.

## 13. Acceptance criteria / Definition of Done

- AC-1: platform-core protocol/network owners implement R1-R9 without a second sent/resend owner.
- AC-2: desktop provider copies are synchronized byte-for-byte with canonical modified sources.
- AC-3: the existing parity ledger/P0/source-closure record this responsibility slice with `validation_status=not-run` and `accepted=false`.
- AC-4: arbitrary multi-request ACK settlement/resend cancellation/container fan-out/fast-history clock correction remain explicitly open.
- AC-5: no Stage-A CI or executable validation is queried or run.

## 14. Release / migration / rollback

No runtime data migration. Release validation is deferred to Stage B; rollback is a normal source revert before candidate freeze.

## 15. Observability / evidence

Evidence is source identity, Git blob identity, authored contracts, canonical-to-desktop blob equality, ledger/P0/source-closure entries, and remote commit/ref readback. Runtime evidence is intentionally absent in Stage A.

## 16. References / provenance

- `Telegram/SourceFiles/mtproto/session_private.cpp@13cbc76ec723386027b5b4dcac79999b791c4614`: `mtpc_msgs_ack`, `SessionPrivate::requestsAcked`.
- `Telegram/SourceFiles/mtproto/scheme/mtproto.tl@76225965bfc794388db241f3f760db8ab79e84f8`: `msgs_ack#62d6b459 msg_ids:Vector<long>`.

## 17. Spec compliance record

| Requirement / AC | Status | Evidence / reason |
| --- | --- | --- |
| R1-R9 | pending | Implementation follows this Spec. |
| AC-1 | pending | Awaiting platform-core implementation. |
| AC-2 | pending | Awaiting desktop synchronization. |
| AC-3 | pending | Awaiting desktop ledger/P0/source-closure update. |
| AC-4 | pending | Must remain explicitly open in ledger/evidence. |
| AC-5 | pending | No executable validation is permitted in Stage A. |
