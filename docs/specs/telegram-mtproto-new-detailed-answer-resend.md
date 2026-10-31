# Telegram MTProto new-detailed-answer resend — Specification

Status: active  
Owner: shared Telegram protocol/network runtime  
Last updated: 2026-11-01  
Related issue/task/PR: Fabushi migration task 7457eb1c-11a3-4626-8563-a3e495872536 round 24

## 1. Context / problem

Fixed upstream `telegramdesktop/tdesktop@f985723b552b30fe1c17d377d57f44e61a15a8c4` handles `msg_new_detailed_info#809db6df` in `SessionPrivate::handleOneReceived`. The message identifies an `answer_msg_id`; if that server answer is already present in recent received-message history Telegram Desktop acknowledges it, otherwise it sends `msg_resend_req` for exactly that answer.

Fabushi already has the source-neutral current-session reconciliation primitive `EstablishedSession::reconcile_detailed_answer` and a receive branch for this constructor, but the branch only reads `answer_msg_id` at a raw byte offset. It therefore does not yet bind the whole fixed-U wire contract (`answer_msg_id:long bytes:int status:int`) or reject truncated/trailing service messages before producing an ACK/resend side effect.

The immediately preceding fixed-U `gzip_packed` responsibility is intentionally not implemented in this slice: the canonical `native/telegram-protocol` crate currently has no direct gzip/deflate dependency and its lock file contains no `flate2` or `miniz_oxide` owner. Under the migration fail-closed rule, gzip nesting/unpacked-budget semantics remain blocked rather than introducing an unreviewed decompression dependency.

## 2. Goal

Make the already-existing single-session `msg_new_detailed_info` receive behavior exact and typed: parse the complete fixed-U object in `telegram-protocol`, reject malformed/trailing data before side effects, and then reuse the existing current-session detailed-answer reconciliation in `telegram-network`.

## 3. Non-goals / out of scope

- `gzip_packed` decompression, `kMaxGzipNesting`, `unpackedBudget`, or adding a compression dependency;
- arbitrary historical `wasSent`, `haveSent`, `_resendingIds`, `toSend`, or `_sentContainers` ownership;
- multi-request callback routing, resend queues, or sent-container fan-out;
- changing the current one-outstanding-RPC / current-ping concurrency model;
- higher-level update/difference propagation;
- treating `bytes` or `status` as a new settlement signal beyond fixed-U's current behavior.

## 4. Requirements

- R1: `telegram-protocol` SHALL own the fixed-U `msg_new_detailed_info#809db6df` wire parser.
- R2: The parser SHALL read exactly `answer_msg_id:long`, `bytes:int`, and `status:int` and SHALL reject truncation, a wrong constructor, and trailing bytes.
- R3: The parser SHALL expose all three fixed-U fields in a typed source-neutral value, even though the current reconciliation uses only `answer_msg_id`.
- R4: `EstablishedSession::scan_rpc_body` SHALL parse this service message before any acknowledgement or resend side effect.
- R5: A parsed `answer_msg_id` already present in the existing recent server-message-id guard SHALL enter the existing acknowledgement accumulator at most once.
- R6: A parsed `answer_msg_id` not present in recent receive history SHALL send exactly one non-content-related `msg_resend_req` through the existing session/auth/transport owner.
- R7: This slice SHALL NOT add historical sent-request/container registries or infer `wasSent/haveSent/_resendingIds/toSend/_sentContainers` semantics.
- R8: The missing gzip dependency and bounded-decompression lifecycle SHALL remain explicitly fail-closed/open.
- R9: Stage-A protocol/network contracts SHALL be authored but not executed.
- R10: Desktop SHALL consume byte-identical synchronized canonical protocol/network sources through the existing provider path.

## 5. Current state

`native/telegram-network/src/lib.rs` already recognizes constructor `0x809db6df` and invokes `reconcile_detailed_answer`, but it accepts any body long enough to read the first i64 after the constructor. The protocol owner has no typed parser for this object. The gzip responsibility has no approved decompression dependency in the canonical protocol crate.

## 6. Target state

`native/telegram-protocol` exports `MsgNewDetailedInfo`, `MSG_NEW_DETAILED_INFO_CONSTRUCTOR`, and `parse_msg_new_detailed_info`. `native/telegram-network` reuses that parser and the existing detailed-answer reconciliation path. No new state owner is created.

## 7. Architecture and ownership boundaries

Wire decoding belongs to the existing `telegram-protocol` owner. Recent received-message identity, ACK accumulation, service-message sending, and current-session lifecycle remain in the existing `EstablishedSession` network owner. Desktop keeps synchronized provider copies only. No second Telegram runtime/provider/message store is introduced.

## 8. Interfaces / contracts / schemas / data flow

`parse_msg_new_detailed_info(&[u8]) -> Result<MsgNewDetailedInfo, ApiRequestError>`.

Data flow: authenticated/decrypted server message -> `scan_rpc_body` -> typed parser -> existing recent server-message-id guard -> either existing ACK accumulator or one existing `build_msg_resend_req` service send -> continue waiting for the current RPC.

## 9. Constraints and non-functional requirements

Preserve current auth key, transport, message-id replay guard, service-message limits, sequence owner, and single-RPC concurrency. Malformed wire input fails closed as `InvalidServiceMessage`. Stage A remains zero CI/runtime validation.

## 10. Failure modes and edge cases

Reject wrong constructor, body shorter than the exact fixed-U object, and trailing bytes. Do not ACK/resend on parse failure. Do not duplicate an ACK already present in the accumulator. Do not broaden to historical request state or gzip.

## 11. Implementation strategy

Add the typed struct/constructor/parser beside existing MTProto service parsers and export them from the protocol library. Replace the network raw-offset branch with the parser. Author parser contracts for exact fields plus truncation/trailing rejection. Preserve the existing reconciliation helper and service send path.

## 12. Verification / test strategy

During Stage A only author source-level unit/contract tests. Do not run build, typecheck, lint, schema validation, generator, unit, contract, integration, E2E, package, deployment, installation, runtime acceptance, GitHub Actions, checks, or statuses.

## 13. Acceptance criteria / Definition of Done

- AC-1: platform-core protocol/network owners implement R1-R9 without a new request/history owner.
- AC-2: malformed/trailing `msg_new_detailed_info` cannot trigger current ACK/resend side effects because full parsing happens first.
- AC-3: desktop provider copies match the canonical modified source blobs byte-for-byte.
- AC-4: parity ledger/P0/source-closure record the new responsibility as `implemented`, `validation_status=not-run`, `accepted=false`, with empty execution evidence.
- AC-5: gzip remains explicitly blocked on the missing bounded-decompression dependency/lifecycle; outbound multi-request histories remain fail-closed.
- AC-6: no Stage-A executable validation is queried or run.

## 14. Release / migration / rollback

No runtime data migration. Release validation is deferred to Stage B. Rollback before candidate freeze is a normal source revert.

## 15. Observability / evidence

Evidence consists of fixed-U source/schema blob identities, canonical source blob identities, authored-but-unrun contracts, desktop/canonical blob equality, ledger/P0/source-closure/evidence records, and remote ref/commit/blob readback.

## 16. References / provenance

- `Telegram/SourceFiles/mtproto/session_private.cpp@13cbc76ec723386027b5b4dcac79999b791c4614`: `mtpc_gzip_packed`, `mtpc_msg_new_detailed_info`, `SessionPrivate::ungzip`, `kMaxGzipNesting`, and `unpackedBudget` receive lifecycle.
- `Telegram/SourceFiles/mtproto/scheme/mtproto.tl@76225965bfc794388db241f3f760db8ab79e84f8`: `msg_new_detailed_info#809db6df answer_msg_id:long bytes:int status:int`.
- fixed upstream commit: `f985723b552b30fe1c17d377d57f44e61a15a8c4`.
- canonical current owners before implementation: `native/telegram-protocol/src/api.rs` and `native/telegram-network/src/lib.rs`.

## 17. Spec compliance record

| Requirement / AC | Status | Evidence / reason |
| --- | --- | --- |
| R1-R10 | passed | Platform-core implementation `5bdf993dc1048ed22194d7f1c0439049e820ee5f`: protocol api `eff20dbc601ca78fa7e6a4662bfeda088706d4f4`, protocol lib `b6ab0397cbad0776b269d965f89427030824ba4c`, network `7f3b140a9becda2305a2f5e252fcea61108597e0`; full `msg_new_detailed_info` parsing precedes the existing reconciliation, contracts are authored but not run, and no historical request/decompression owner was added. |
| AC-1 | passed | Existing protocol/network owners implement the exact typed parser and current-session reconciliation at `5bdf993dc1048ed22194d7f1c0439049e820ee5f`. |
| AC-2 | passed | `parse_msg_new_detailed_info` rejects wrong/truncated/trailing objects before `reconcile_detailed_answer` can emit an ACK or resend. |
| AC-3 | passed | Desktop implementation `3b37ff49e7c3b100af12206ab88662f4d83959c2` synchronizes provider blobs byte-for-byte to `eff20dbc601ca78fa7e6a4662bfeda088706d4f4`, `b6ab0397cbad0776b269d965f89427030824ba4c`, and `7f3b140a9becda2305a2f5e252fcea61108597e0`. |
| AC-4 | passed | Desktop bookkeeping commit `f50194055cf2db53b7f8be3c4d9e4793465511b4` records `TDRP-R9-MTPROTO-NEW-DETAILED-INFO-WIRE-001` as implemented with empty execution evidence, `validation_status=not-run`, `accepted=false`, and `responsibilities_total=66`. |
| AC-5 | passed | Gzip remains explicitly blocked because canonical `Cargo.toml@d8a5261034b1eb395d12149a6bcc34cd7e371b73` / `Cargo.lock@97ad9046ce8926764258c978664cd8cd92fce503` have no approved `flate2`/\`miniz_oxide` owner or bounded decompression lifecycle; `wasSent/haveSent/_resendingIds/toSend/_sentContainers` remain open/fail-closed. |
| AC-6 | passed | Round 24 performed source/ref/blob reads, authored contracts, and CAS writes only; no Actions/status/check query or trigger and no build, lint, validator, generator, test, package, deployment, installation, or runtime acceptance was performed. |
