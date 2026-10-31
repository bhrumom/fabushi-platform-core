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


## 18. Round 37 request-side extension — fixed-U ping exact serializer

At live HEAD `eaf1eeefdde1401d820d9fecd15da3b4aa9cfde5`, the canonical protocol owner already has the strict receive-side `Pong` parser and the separately implemented `ping_delay_disconnect` request builder, but fixed-U `ping#7abe77ec ping_id:long = Pong` still has no canonical exact serializer. Fixed-U `SessionPrivate::tryToSend` chooses ordinary `MTPPing` only from connection/main-session state (`sendOnlyFirstPing || !isMainSession`) and otherwise chooses `MTPPing_delay_disconnect`; that scheduling/selection policy is session lifecycle, not wire shape, and is intentionally not inferred here.

Additional requirements:

- R9: The canonical protocol owner SHALL export `PING_CONSTRUCTOR = 0x7abe77ec`.
- R10: The canonical protocol owner SHALL expose `build_ping(ping_id: i64) -> Vec<u8>`.
- R11: The builder SHALL encode exactly the request constructor followed by `ping_id:long`, little-endian, for exactly 12 bytes.
- R12: The builder SHALL preserve the complete signed `i64` domain without applying timer, connection, main-session, retry, correlation, or reconnect policy.
- R13: The request builder and its contracts SHALL remain structurally separate from `parse_pong`, `PONG_CONSTRUCTOR`, and current-ping receive correlation/settlement.
- R14: Authored-only request contracts SHALL cover exact layout, a representative value, `i64::MIN` / `i64::MAX`, and request/Pong-parser separation; they SHALL NOT be executed in Stage A.
- R15: Exact request serialization SHALL have no socket, timer, session, request-history, ACK, resend, persistence, or response-settlement side effects.
- R16: Live ordinary-ping scheduling/selection, message-id assignment, send/ACK settlement, and timeout/reconnect lifecycle SHALL remain fail-closed unless and until the existing network/session owner can represent the fixed-U semantics precisely without adding arbitrary `wasSent/haveSent/_resendingIds/toSend/_sentContainers` history.

Additional acceptance criteria:

- AC-6: Exact `ping` request serialization is owned by the existing source-neutral `telegram-protocol` owner.
- AC-7: The serializer emits exactly 12 bytes and preserves signed `ping_id` without policy transformation.
- AC-8: Request serialization stays separate from `Pong` response parsing and current-ping correlation/settlement.
- AC-9: Desktop protocol provider sources are synchronized byte-for-byte from canonical sources; authored request contracts remain not-run and `accepted=false`.
- AC-10: Ordinary-ping timer/session scheduling and the broader historical sent/resend/container lifecycle remain explicit fail-closed scope; gzip and complete difference/differenceSlice decoding/application remain untouched.


## 19. Round 37 request-side source-level compliance readback

Canonical implementation commit: `d6ef74975d3541ddb4d187bfb5641773dc01a32d`.  
Desktop provider synchronization commit: `36b425e547509f334453ac5ebf09033e366fe6ae`.  
Desktop bookkeeping commit: `d33003c70384b7f80769982b7c684ec9ea6096ed`.  
Canonical and desktop protocol blobs: api `714c24a5f1727e13693008ea42676831b3e8d3fd`, lib `ef44b1bb7fcac0183f1b83cca8c9d994ecc8ce9c`.

- R9: satisfied by source inspection — `PING_CONSTRUCTOR` is exported as fixed-U `0x7abe77ec`.
- R10: satisfied by source inspection — `build_ping(ping_id: i64) -> Vec<u8>` is public and re-exported by the protocol crate.
- R11: satisfied by source inspection — the builder writes only the constructor then one little-endian `i64`, yielding the exact 12-byte contract.
- R12: satisfied by source inspection — `ping_id` is written directly with no validation, clamp, timer, connection, retry, main-session, correlation, or reconnect transformation.
- R13: satisfied by source inspection — request serialization is a distinct builder; `parse_pong` and current-ping receive correlation/settlement are unchanged and remain separate receive-side responsibilities.
- R14: satisfied as an authored-only Stage-A artifact — exact-layout, signed-boundary, and request/Pong-parser separation contracts exist but were not run.
- R15: satisfied by ownership/source inspection — the builder has no socket, timer, session, request-history, ACK/resend, persistence, timeout/reconnect, or response-settlement side effect.
- R16: satisfied as an explicit fail-closed boundary — fixed-U ordinary-ping scheduling/selection, message-id assignment, send/ACK settlement and timeout/reconnect remain unclaimed; arbitrary `wasSent/haveSent/_resendingIds/toSend/_sentContainers` history was not introduced.

AC-6 through AC-9 are satisfied at source/implementation/bookkeeping level. AC-10 is satisfied for the fail-closed ownership boundary and remains **blocked/not-run** for executable validation: `validation_status=not-run`, `accepted=false`. TLS camouflage, gzip, complete difference/differenceSlice decoding/application, broader source/legal closure, Stage-B evidence and independent release acceptance remain open. No build, typecheck, lint, schema validator, generator, unit/contract/integration/E2E, package, deployment, installation, GitHub Actions, checks, or statuses were run or queried in this review.


## 20. Round 63 adjacent receive-side revalidation — fixed-U Pong exact wire

At live checkpoint `3e24ea732acd2bcb2fe5bf2e5257f275643384de`, fixed U remains `telegramdesktop/tdesktop@f985723b552b30fe1c17d377d57f44e61a15a8c4`. Rereading `mtproto.tl@76225965bfc794388db241f3f760db8ab79e84f8` and `session_private.cpp@13cbc76ec723386027b5b4dcac79999b791c4614` confirms the next shipping service-message branch after `new_session_created` is `pong#347773c5 msg_id:long ping_id:long = Pong`.

This is a source-level **no-production-delta** revalidation. The current canonical protocol `api.rs@ebfb3670b5cd097b9ce288782c7954f5cfe9fc43` already exposes `PONG_CONSTRUCTOR`, typed `Pong`, and `parse_pong`; it consumes constructor 4 + `msg_id` 8 + `ping_id` 8 = exactly 20 bytes, preserves both signed `i64` values, rejects constructor mismatch or truncation through the bounded `TlReader`, and rejects any trailing bytes. The current desktop shipping network blob `1fc1b1f69e3e37377400d078f916599e0c7b20bc` routes `PONG_CONSTRUCTOR` through that parser before exact current-ping `msg_id` + `ping_id` correlation.

Round-63 authored-only external contracts SHALL pin the 20-byte object, offsets `0..4 / 4..12 / 12..20`, signed `i64::MIN` / `i64::MAX` preservation across the two fields, every truncation cut `0..19`, wrong-constructor rejection, and strict EOF. These contracts SHALL NOT be executed during Stage A.

The wire-only boundary remains intentionally separate from fixed-U `SessionPrivate::wasSent`, `requestsFixTimeSalt`, `requestsAcked`, arbitrary `haveSent/_resendingIds/toSend/_sentContainers` history, connection scheduling, auth-key binding/persistence, `gzip_packed`, and complete difference decoding/application. The existing single-current-ping correlation is retained; no wider historical request owner is inferred.

The existing ledger responsibility `TDRP-R9-MTPROTO-SINGLE-PING-PONG-001` remains the sole row and stays `implemented / validation_status=not-run / accepted=false`. No GitHub Actions/status/check query or trigger, build, typecheck, lint, schema validator, generator, unit/contract/integration/E2E test, package, deployment, installation, or runtime acceptance is permitted or claimed for this Stage-A revalidation.


## 21. Round 63 final source-level compliance

Remote desktop readback closes this adjacent fixed-U `Pong` wire revalidation at source level with **no production delta**.

- Canonical production protocol api/lib/network blobs remain `ebfb3670b5cd097b9ce288782c7954f5cfe9fc43` / `abab39e42f100972484191f0e52e9edd9b88c6cf` / `1fc1b1f69e3e37377400d078f916599e0c7b20bc`.
- The authored-only canonical external contract is commit `a559782b390f43d5caa28d8eaba1f99a9292059b`, blob `7672b1908bdf8810592e86f8427fd85c49089d7f`.
- Desktop mirrors that contract byte-identically and synchronizes evidence/ledger/P0/source-closure in commit `a05f05cacac7402762f91915adb5664b6178ec7d`.
- Desktop remote readback reports contract / ledger / P0 / source-closure / round94 evidence blobs as `7672b1908bdf8810592e86f8427fd85c49089d7f` / `a341fa6afc1ec4cb8d2936f9cddc8d6e01fb256e` / `c76503ef4ce594af2429d7f773f6d3e0b75756f6` / `896210cbb58bdbec3cd1385dab2b73984b74e7fc` / `1e359f5b9cab728da6e1ab9b7c1f21e27817d80e`.
- The existing `TDRP-R9-MTPROTO-SINGLE-PING-PONG-001` row remains `implemented / validation_status=not-run / accepted=false`; no duplicate responsibility is created.
- Fixed-U historical `wasSent/requestsFixTimeSalt/requestsAcked`, arbitrary multi-request sent/resend/container state, connection scheduling, auth-key binding/persistence, gzip, complete difference decoding/application, broader source/legal closure, Stage-B executable evidence, and independent release ACCEPT remain open or outside this wire-only claim.

No GitHub Actions/status/check query or trigger, build, typecheck, lint, schema validator, generator, unit/contract/integration/E2E test, package, deployment, installation, or runtime acceptance was performed.
