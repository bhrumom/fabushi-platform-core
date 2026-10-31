# Telegram MTProto single-RPC message-container traversal — Specification

Status: active  
Owner: shared Telegram protocol/network runtime  
Last updated: 2026-11-01  
Related issue/task/PR: Fabushi migration task 7457eb1c-11a3-4626-8563-a3e495872536 round 23

## 1. Context / problem

Fixed upstream `telegramdesktop/tdesktop@f985723b552b30fe1c17d377d57f44e61a15a8c4` handles `mtpc_msg_container` in `SessionPrivate::handleOneReceived`. Each contained message carries `msg_id`, `seq_no`, a byte length, and an inner TL object. Telegram Desktop validates the inner message id, records an acknowledgement for odd sequence numbers, and recursively handles the contained object.

Fabushi already has receive-side container traversal in `native/telegram-network`, including current-RPC and current-ping recursion, but the wire shape is duplicated as fixed-offset reads in two branches and accepts contained bodies that fixed U rejects when shorter than one TL word or not word-aligned. The existing `telegram-protocol` owner is the correct place for a bounded typed container parser; the existing `EstablishedSession` remains the only live current-request/session owner.

## 2. Goal

Represent exactly the fixed-U receive-side message-container subset that the current single-outstanding-RPC / single-current-ping owners can carry: bounded typed parsing, exact full-buffer consumption, inner-message metadata preservation, existing message-id validation, odd-sequence acknowledgement accumulation on the current RPC path, and recursive current RPC/ping service-message correlation.

## 3. Non-goals / out of scope

- Telegram Desktop `_sentContainers` or any sent-container history;
- arbitrary historical `wasSent`, `haveSentMap`, `_resendingIds`, `toSendMap`, callback registries, or multi-request settlement;
- container-wide resend fan-out for historical outbound requests;
- introducing a second session, request registry, sent queue, transport, provider, or message store;
- `gzip_packed` decompression, nesting limits, or unpacked-byte budgets;
- higher-level update/difference propagation;
- changing the existing single-RPC/single-ping concurrency model.

These responsibilities remain open/fail-closed until their canonical owners can represent them exactly.

## 4. Requirements

- R1: `telegram-protocol` SHALL own the fixed-U `msg_container#73f1f8dc` receive-side wire parser.
- R2: The parser SHALL bound the contained-message count to the existing network limit of 1024 and reject negative/out-of-range counts.
- R3: Each contained message SHALL expose `message_id`, `sequence_number`, and opaque inner object bytes.
- R4: Each inner body length SHALL be at least one TL word, divisible by four, non-negative, in-bounds, and the complete container input SHALL be consumed exactly.
- R5: `EstablishedSession::scan_rpc_body` SHALL traverse parsed entries and validate each inner server message id through the existing `MessageIdGuard` before recursive handling.
- R6: On the current RPC path, odd inner sequence numbers SHALL enter the existing acknowledgement accumulator; even sequence numbers SHALL not.
- R7: A nested service/result message SHALL affect only the existing current RPC according to its existing exact-correlation rules; no historical request lookup SHALL be added.
- R8: `EstablishedSession::process_service_body` SHALL use the same typed parser for current-ping container traversal and retain existing exact pong correlation.
- R9: Malformed containers SHALL fail closed as the existing `InvalidServiceMessage` boundary.
- R10: Stage-A parser/traversal contracts SHALL be authored but not executed.
- R11: Desktop SHALL consume synchronized canonical protocol/network sources through the existing provider path chain.
- R12: `wasSent/haveSent/_resendingIds/toSend/_sentContainers` and gzip responsibilities SHALL remain explicitly open/fail-closed.

## 5. Current state

Both network receive paths manually read the constructor, signed count, entry metadata, body length, and offsets. They already cap count at 1024, recursively dispatch inner messages, and validate inner message ids. The RPC path also adds odd-sequence inner ids to its acknowledgement accumulator. Body length currently needs only to be non-negative and in-bounds, which is weaker than fixed U's minimum-one-word and word-alignment checks.

## 6. Target state

`native/telegram-protocol` returns a bounded `MessageContainer` with typed `MessageContainerEntry` values. `native/telegram-network` performs only lifecycle/state responsibilities: validate each parsed inner message id, collect current-RPC acknowledgements from sequence parity, and recursively apply the existing current RPC/ping service handlers.

## 7. Architecture and ownership boundaries

Wire shape and bounded decoding belong to `native/telegram-protocol`. Current request/ping state, message-id replay protection, acknowledgement emission, and recursive service handling remain in the existing `EstablishedSession`. Desktop synchronizes these canonical sources into its existing provider crates. No new Telegram-specific product root or parallel runtime is introduced.

## 8. Interfaces / contracts / schemas / data flow

`parse_message_container(&[u8]) -> Result<MessageContainer, ApiRequestError>` returns entries with `message_id: i64`, `sequence_number: i32`, and opaque `body: Vec<u8>`.

RPC data flow: authenticated/decrypted outer message -> `scan_rpc_body` -> typed container parser -> inner message-id validation -> odd-sequence current acknowledgement accumulation -> recursive `scan_rpc_body` -> existing exact current-request settlement/recovery.

Ping data flow: authenticated/decrypted outer message -> `process_service_body` -> typed container parser -> inner message-id validation -> recursive `process_service_body` -> existing exact current-ping pong correlation.

## 9. Constraints and non-functional requirements

Preserve the existing encrypted/authenticated boundary, replay guard, service-message limits, transport, retry budgets, source-neutral Rust ownership, and current single-request concurrency. No background worker or persistence is added. Stage A remains zero CI/runtime validation.

## 10. Failure modes and edge cases

Reject wrong constructor, truncated count/metadata, negative or >1024 count, body lengths below four bytes, non-word-aligned lengths, arithmetic/buffer overrun, and trailing bytes after the declared entries. Nested unrelated results remain non-settling under existing correlation rules. Historical outbound-container semantics must not be inferred from the receive-side parser.

## 11. Implementation strategy

Add the constructor, typed container structs, parser, and parser contracts to `native/telegram-protocol`; export them from the existing library surface. Replace both network fixed-offset container loops with the parser while preserving message-id validation, current-RPC sequence-parity acknowledgement accumulation, and recursive outcomes. Add a pure sequence-parity helper contract. Synchronize the same modified source files into the desktop provider path.

## 12. Verification / test strategy

During Stage A only author contracts for: successful multi-entry parse, minimum/alignment/count/trailing-data rejection, and odd/even current-RPC acknowledgement parity. Do not execute build, lint, generator, unit, contract, integration, packaged, deployment, installation, runtime, or GitHub Actions/status validation.

## 13. Acceptance criteria / Definition of Done

- AC-1: platform-core protocol/network owners implement R1-R10 without any second request/sent/container-history owner.
- AC-2: desktop provider copies are synchronized byte-for-byte with canonical modified sources.
- AC-3: the existing parity ledger/P0/source-closure record only the bounded current-RPC/current-ping receive-container subset as implemented, with `validation_status=not-run`, `accepted=false`, and empty execution evidence.
- AC-4: outbound/multi-request `wasSent/haveSent/_resendingIds/toSend/_sentContainers` and `gzip_packed` remain explicitly open/fail-closed.
- AC-5: no Stage-A CI or executable validation is queried or run.

## 14. Release / migration / rollback

No runtime data migration. Release validation is deferred to Stage B; rollback is a normal source revert before candidate freeze.

## 15. Observability / evidence

Evidence is fixed-U source/blob identity, canonical Git blob identity, authored contracts, canonical-to-desktop blob equality, ledger/P0/source-closure entries, and remote ref/commit/blob readback. Runtime evidence is intentionally absent in Stage A.

## 16. References / provenance

- `Telegram/SourceFiles/mtproto/session_private.cpp@13cbc76ec723386027b5b4dcac79999b791c4614`: `mtpc_msg_container` receive-side branch.
- fixed upstream commit: `f985723b552b30fe1c17d377d57f44e61a15a8c4`.
- existing canonical owner before this slice: `native/telegram-network/src/lib.rs` message-container branches.

## 17. Spec compliance record

| Requirement / AC | Status | Evidence / reason |
| --- | --- | --- |
| R1-R12 | passed | Source review at platform-core implementation `98a2ff425407725571966a4ed17b1643fba87b4d`: bounded typed `parse_message_container`, 1024-entry cap, minimum/aligned inner-body checks, exact full-buffer consumption, existing MessageIdGuard validation, current-RPC odd-sequence ACK accumulation, current-RPC/current-ping recursive dispatch, no outbound sent-container/request-history owner, and contracts authored but not run. |
| AC-1 | passed | Canonical blobs: protocol api `14c282bfc20a33446c69a47db4fdd6f0b887348a`, protocol lib `f21d38495e5a7f5c6e2a35f28a633ac6e3433d4b`, network `bab8893bd8a2a0632014d826503a8091cfe64d46`. |
| AC-2 | passed | Desktop implementation `e8091133b85eb7e368839e6c9b09547620be17c2`; remote provider blobs match the canonical three blob identities. |
| AC-3 | passed | Desktop ledger/P0/source-closure/evidence commit `b3ed649d1fc8d0ed1af6d0229489df82ecd42a78`; row `TDRP-R9-MTPROTO-SINGLE-RPC-MESSAGE-CONTAINER-001` is implemented with empty execution evidence and records `validation_status=not-run`, `accepted=false`. |
| AC-4 | passed | Desktop ledger/evidence explicitly retain outbound `wasSent/haveSent/_resendingIds/toSend/_sentContainers`, multi-request fan-out, and `gzip_packed` as open/fail-closed. |
| AC-5 | passed | Round 23 performed Git/ref/blob/source reads and safe writes only; no Actions/status/check query or trigger and no build, lint, generator, test, package, deployment, installation, or runtime acceptance was performed. |
