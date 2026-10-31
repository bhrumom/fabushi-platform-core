# Telegram MTProto single-ping pong — Specification

Status: active  
Owner: shared Telegram protocol/network runtime  
Last updated: 2026-11-01  
Related issue/task/PR: Fabushi migration task 7457eb1c-11a3-4626-8563-a3e495872536 round 21

## 1. Context / problem

Fixed upstream `telegramdesktop/tdesktop@f985723b552b30fe1c17d377d57f44e61a15a8c4` handles `mtpc_pong` in `SessionPrivate::handleOneReceived`. The fixed-U schema is `pong#347773c5 msg_id:long ping_id:long = Pong`. Telegram Desktop accepts a pong only in relation to a recently sent message and clears the live ping only when the returned `ping_id` matches that ping.

Fabushi's canonical `EstablishedSession::send_ping` has exactly one outstanding ping. Before this slice, the network owner reads pong fields by fixed offsets and compares both values, but the protocol owner has no bounded pong parser and the service branch can accept trailing bytes after the two required fields.

## 2. Goal

Represent exactly the fixed-U pong semantics that the existing single-outstanding-ping owner can carry: fully parse and validate `pong`, correlate both `msg_id` and `ping_id` with the current ping, and settle only that ping.

## 3. Non-goals / out of scope

- arbitrary historical `haveSentMap` lookup or acknowledged-id retention;
- `_resendingIds`, `toSendMap`, `_sentContainers`, or `_stateAndResendRequests`;
- multi-ping history or concurrent ping tracking;
- transport ACK queue fan-out or resend cancellation;
- introducing a second runtime, session, transport, request queue, or provider.

Those responsibilities remain open/fail-closed until a canonical owner can represent them without duplicating state truth.

## 4. Requirements

- R1: The canonical protocol owner SHALL parse fixed-U `pong#347773c5 msg_id:long ping_id:long` using the existing bounded TL reader.
- R2: The parser SHALL reject a wrong constructor, truncation, malformed fields, or trailing bytes.
- R3: The canonical network ping path SHALL route `PONG_CONSTRUCTOR` through that parser instead of fixed-offset field reads.
- R4: A pong SHALL settle the current ping only when both `msg_id == request_message_id` and `ping_id == current_ping_id`.
- R5: A mismatched message id or ping id SHALL fail closed through the existing `NetworkError::PingMismatch` path.
- R6: No historical sent/resend/container queue state SHALL be added.
- R7: Stage-A parser/correlation contracts SHALL be authored but not executed.
- R8: Desktop SHALL consume synchronized canonical protocol/network sources through the existing provider path chain.

## 5. Current state

The network owner already compares the two fixed-width fields for the current ping, but parses them locally by offsets. The protocol owner has no `parse_pong` function, so malformed trailing payload is not rejected by the protocol contract.

## 6. Target state

`native/telegram-protocol` owns strict pong decoding; `native/telegram-network::EstablishedSession` owns current-ping correlation and settlement. A valid exact pong returns the existing service `Pong` outcome. Wrong, truncated, or trailing payload fails closed without changing ping state.

## 7. Architecture and ownership boundaries

Protocol shape belongs to `native/telegram-protocol`. Current ping state belongs to the existing `EstablishedSession`. Desktop synchronizes the canonical source files into its existing provider crates. No new account/session/network owner is introduced.

## 8. Interfaces / contracts / schemas / data flow

`parse_pong(&[u8]) -> Result<Pong, ApiRequestError>` decodes constructor, message id and ping id and rejects trailing bytes. `pong_matches_request(pong, request_message_id, ping_id)` expresses the only current-state correlation this owner can represent.

## 9. Constraints and non-functional requirements

Preserve the existing encrypted/authenticated service-message boundary, sequence/message-id validation, ping timeout behavior and source-neutral Rust ownership. No background queue or persistence is added. Stage A remains zero CI/runtime validation.

## 10. Failure modes and edge cases

Wrong constructor, short payload and trailing bytes fail closed. Either id mismatch returns `PingMismatch`. A matching pong settles only the current ping. Historical sent-message correlation beyond the current ping is intentionally not inferred.

## 11. Implementation strategy

Add `Pong` and `parse_pong` to the protocol API, export them, replace fixed-offset pong reads in the network service branch with the protocol parser, add a pure correlation helper, and author parser/correlation tests without executing them.

## 12. Verification / test strategy

Author unit/contract tests only during Stage A. Do not execute build, lint, generator, unit, contract, integration, packaged, deployment, installation or runtime checks, and do not query Actions/status/check results.

## 13. Acceptance criteria / Definition of Done

- AC-1: platform-core protocol/network owners implement R1-R7 without any second sent/resend owner.
- AC-2: desktop provider copies are synchronized byte-for-byte with canonical modified sources.
- AC-3: the existing parity ledger/P0/source-closure record the pong responsibility with `validation_status=not-run` and `accepted=false`.
- AC-4: `haveSent/_resendingIds/toSend/_sentContainers` and other multi-request history remain explicitly open/fail-closed.
- AC-5: no Stage-A CI or executable validation is queried or run.

## 14. Release / migration / rollback

No runtime data migration. Release validation is deferred to Stage B; rollback is a normal source revert before candidate freeze.

## 15. Observability / evidence

Evidence is fixed-U source identity, Git blob identity, authored contracts, canonical-to-desktop blob equality, ledger/P0/source-closure entries, and remote ref/commit/blob readback. Runtime evidence is intentionally absent in Stage A.

## 16. References / provenance

- `Telegram/SourceFiles/mtproto/session_private.cpp@13cbc76ec723386027b5b4dcac79999b791c4614`: `mtpc_pong`.
- `Telegram/SourceFiles/mtproto/scheme/mtproto.tl@76225965bfc794388db241f3f760db8ab79e84f8`: `pong#347773c5 msg_id:long ping_id:long = Pong;`.

## 17. Spec compliance record

| Requirement / AC | Status | Evidence / reason |
| --- | --- | --- |
| R1-R8 | passed | Source review at platform-core implementation `bf6f063b229be43597bddc4231baab24d06a09bd`: bounded `parse_pong`, strict trailing-byte rejection, exact current message-id + ping-id correlation, existing PingMismatch/InvalidServiceMessage fail-closed paths, no multi-request sent/ping owner, and contracts authored but not run. |
| AC-1 | passed | Canonical blobs: protocol api `32a7610bfc74835734c1e5c4385e4eb4cb2ee1f3`, protocol lib `ffbabc8f66112c2321be75a34928b086d6adf57f`, network `e0bf1b5f286ad0f8bc45a36e86074e19b7dde560`. |
| AC-2 | passed | Desktop implementation `3e363ccdaf287878f457797442aa97e8b597384b`; remote readback confirmed the same three blob identities on the existing provider path chain. |
| AC-3 | passed | Desktop ledger/P0/source-closure/evidence commit `770510d554552d5d4aa2f2b69f3d62612e3e33ad`; row `TDRP-R9-MTPROTO-SINGLE-PING-PONG-001` is implemented with empty execution evidence and records `validation_status=not-run`, `accepted=false`. |
| AC-4 | passed | Desktop ledger/evidence explicitly retain arbitrary `haveSent/_resendingIds/toSend/_sentContainers`, historical sent-message lookup and multi-ping history as open/fail-closed. |
| AC-5 | passed | Round 21 performed Git/ref/tree/blob/source reads and safe writes only; no Actions/status/check query or trigger and no build, lint, generator, test, package, deployment, installation, or runtime acceptance was performed. |
