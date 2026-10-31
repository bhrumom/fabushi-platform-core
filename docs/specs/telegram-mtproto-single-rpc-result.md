# Telegram MTProto single-RPC result correlation — Specification

Status: active  
Owner: shared Telegram protocol/network runtime  
Last updated: 2026-11-01  
Related issue/task/PR: Fabushi migration task 7457eb1c-11a3-4626-8563-a3e495872536 round 22

## 1. Context / problem

Fixed upstream `telegramdesktop/tdesktop@f985723b552b30fe1c17d377d57f44e61a15a8c4` handles `mtpc_rpc_result` in `SessionPrivate::handleOneReceived`. The fixed-U envelope is `rpc_result#f35c6d01 req_msg_id:long result:Object = RpcResult` (parsed manually by Telegram Desktop). Telegram Desktop then consults its historical sent-request map and acknowledgement machinery before handing the response to the higher layer.

Fabushi's canonical `EstablishedSession::invoke_raw` intentionally has exactly one outstanding content RPC. Before this slice, the network owner already compares the fixed-offset `req_msg_id` with that current request, but the protocol owner has no bounded typed parser for the RPC result envelope and the correlation rule is embedded inline.

## 2. Goal

Represent exactly the fixed-U RPC-result semantics that the existing single-outstanding-RPC owner can carry: parse the `rpc_result` envelope, preserve the opaque result object bytes, correlate only the current `request_message_id`, and settle only that current call.

## 3. Non-goals / out of scope

- arbitrary historical `wasSent` / `haveSentMap` lookup;
- `requestsAcked` bookkeeping for multiple requests;
- `_resendingIds`, `toSendMap`, `_sentContainers`, or `_stateAndResendRequests`;
- callback/request-id routing for multiple concurrent RPCs;
- gzip-unpack ownership beyond the existing higher-level result decoding path;
- introducing a second sent queue, runtime, provider, transport, session, or message store.

Those responsibilities remain open/fail-closed until the canonical architecture has a single source of truth that can represent them.

## 4. Requirements

- R1: The canonical protocol owner SHALL parse fixed-U `rpc_result#f35c6d01 req_msg_id:long result:Object` as a bounded envelope.
- R2: The parser SHALL reject a wrong constructor, truncated request id, an empty result object, or a result body that is not TL word-aligned.
- R3: The result object bytes SHALL remain opaque to the envelope parser and be returned unchanged for the existing method-specific decoding path.
- R4: The canonical network RPC path SHALL route `RPC_RESULT_CONSTRUCTOR` through that parser instead of fixed-offset envelope reads.
- R5: A parsed result SHALL settle the current call only when `req_msg_id == current request_message_id`.
- R6: A well-formed result for an unrelated request id SHALL not settle or mutate the current call; it SHALL continue waiting.
- R7: A malformed RPC-result envelope SHALL fail closed as the existing invalid-service-message error.
- R8: No historical sent/resend/container/request-routing state SHALL be introduced.
- R9: Stage-A parser/correlation contracts SHALL be authored but not executed.
- R10: Desktop SHALL consume synchronized canonical protocol/network sources through the existing provider path chain.

## 5. Current state

The network owner reads `req_msg_id` at offset 4 and treats bytes after offset 12 as the result body when non-empty. It has the correct single-current-request comparison, but envelope validation and typed ownership are not centralized in `telegram-protocol`.

## 6. Target state

`native/telegram-protocol` owns strict RPC-result envelope decoding. `native/telegram-network::EstablishedSession` owns current-request correlation and settlement. The opaque result bytes continue through the existing `RpcResult` return path without a second request registry.

## 7. Architecture and ownership boundaries

Protocol shape belongs to `native/telegram-protocol`. The current outstanding request belongs to the existing `EstablishedSession::invoke_raw`. Desktop synchronizes those canonical source files into its existing provider crates. No new account/session/network owner is introduced.

## 8. Interfaces / contracts / schemas / data flow

`parse_rpc_result(&[u8]) -> Result<RpcResultEnvelope, ApiRequestError>` returns the fixed-U `request_message_id` plus the remaining word-aligned result object bytes. `rpc_result_matches_request(response_request_message_id, request_message_id)` expresses the only request correlation the current owner can represent exactly.

## 9. Constraints and non-functional requirements

Preserve the existing encrypted/authenticated service-message boundary, message-id/sequence checks, retry budget, transport, and source-neutral Rust ownership. No background queue or persistence is added. Stage A remains zero CI/runtime validation.

## 10. Failure modes and edge cases

Wrong constructor, short request id, missing result bytes, and non-word-aligned result bytes fail closed. A valid result with another request id is ignored for the current call. Historical routing beyond the current outstanding RPC is intentionally not inferred.

## 11. Implementation strategy

Add the fixed-U constructor, `RpcResultEnvelope`, and `parse_rpc_result` to the protocol API; export them; replace fixed-offset `rpc_result` envelope reads in the network owner with the parser; add a pure correlation helper; and author parser/correlation contracts without executing them.

## 12. Verification / test strategy

Author unit/contract tests only during Stage A. Do not execute build, lint, generator, unit, contract, integration, packaged, deployment, installation, or runtime checks, and do not query Actions/status/check results.

## 13. Acceptance criteria / Definition of Done

- AC-1: platform-core protocol/network owners implement R1-R9 without any second sent/resend/request-history owner.
- AC-2: desktop provider copies are synchronized byte-for-byte with canonical modified sources.
- AC-3: the existing parity ledger/P0/source-closure record the single-RPC-result responsibility with `validation_status=not-run` and `accepted=false`.
- AC-4: `haveSent/_resendingIds/toSend/_sentContainers` and other multi-request history remain explicitly open/fail-closed.
- AC-5: no Stage-A CI or executable validation is queried or run.

## 14. Release / migration / rollback

No runtime data migration. Release validation is deferred to Stage B; rollback is a normal source revert before candidate freeze.

## 15. Observability / evidence

Evidence is fixed-U source identity, Git blob identity, authored contracts, canonical-to-desktop blob equality, ledger/P0/source-closure entries, and remote ref/commit/blob readback. Runtime evidence is intentionally absent in Stage A.

## 16. References / provenance

- `Telegram/SourceFiles/mtproto/session_private.cpp@13cbc76ec723386027b5b4dcac79999b791c4614`: `mtpc_rpc_result`.
- `Telegram/SourceFiles/mtproto/scheme/mtproto.tl@76225965bfc794388db241f3f760db8ab79e84f8`: `rpc_result#f35c6d01 req_msg_id:long result:Object = RpcResult` (manual parser declaration).

## 17. Spec compliance record

| Requirement / AC | Status | Evidence / reason |
| --- | --- | --- |
| R1-R10 | blocked | Pending implementation in this round. |
| AC-1 | blocked | Pending canonical implementation. |
| AC-2 | blocked | Pending desktop synchronization. |
| AC-3 | blocked | Pending ledger/P0/source-closure update. |
| AC-4 | blocked | Must remain explicitly fail-closed. |
| AC-5 | blocked | Will be reviewed after Stage-A source writes; no executable verification is permitted. |
