# Telegram MTProto future-salts wire validation — Specification

Status: active  
Owner: shared Telegram protocol/network runtime  
Last updated: 2026-11-01  
Related task: Fabushi migration task 7457eb1c-11a3-4626-8563-a3e495872536 round 27

## Context

Fixed upstream `telegramdesktop/tdesktop@f985723b552b30fe1c17d377d57f44e61a15a8c4` declares `future_salts#ae500895` as `req_msg_id:long now:int salts:vector<future_salt>`, where each `future_salt#0949d9dc` contains `valid_since:int valid_until:int salt:long`.

Fabushi's canonical `EstablishedSession::scan_rpc_body` currently recognizes constructor `ae500895`, reads only the raw `req_msg_id` at byte offset 4, and—when it matches the one outstanding RPC—returns the entire unvalidated body as the RPC result. A body with a valid constructor/request id but truncated or malformed `now`, vector header, nested `future_salt`, or trailing bytes can therefore settle the call without complete fixed-U wire validation.

## Fixed-U provenance

- `Telegram/SourceFiles/mtproto/scheme/mtproto.tl@76225965bfc794388db241f3f760db8ab79e84f8`
  - `future_salt#0949d9dc valid_since:int valid_until:int salt:long = FutureSalt`;
  - `future_salts#ae500895 req_msg_id:long now:int salts:vector<future_salt> = FutureSalts`;
  - `get_future_salts#b921bd04 num:int = FutureSalts`.
- `Telegram/SourceFiles/mtproto/session_private.cpp@13cbc76ec723386027b5b4dcac79999b791c4614`
  - adjacent service-message handling uses generated TL objects before acting on decoded fields; no independent multi-request history is required merely to validate a current response object.

## Requirements

- R1: The canonical protocol owner SHALL export `FUTURE_SALT_CONSTRUCTOR`, `FUTURE_SALTS_CONSTRUCTOR`, typed `FutureSalt` / `FutureSalts`, and `parse_future_salts(&[u8])`.
- R2: The parser SHALL decode exactly `req_msg_id:long`, `now:int`, the TL vector constructor/count, and every nested `future_salt#0949d9dc` field.
- R3: Wrong top-level constructor, wrong vector constructor, wrong nested constructor, truncation at any field, impossible declared vector length for the remaining fixed-size entries, and trailing bytes SHALL fail closed.
- R4: Structural validation SHALL occur before allocating from an untrusted declared vector length; the declared count must fit the remaining bytes for fixed-size `future_salt` entries.
- R5: `EstablishedSession::scan_rpc_body` SHALL invoke the typed parser before current-request matching or RPC settlement.
- R6: A valid `future_salts.req_msg_id` SHALL settle only the one currently outstanding RPC when it exactly matches that request id.
- R7: A valid unrelated `req_msg_id` SHALL be ignored for the current call and SHALL NOT mutate retry/session state.
- R8: The caller-visible `RpcResult.body` SHALL remain the original exact validated `future_salts` TL object; this slice does not add a second result/runtime abstraction.
- R9: Protocol parsing, current-request correlation, authenticated session transport, and message-id guards SHALL remain in the existing protocol/network owners. This slice SHALL NOT introduce `wasSent/haveSent/_resendingIds/toSend/_sentContainers` history, gzip decompression, or full updates-difference decoding.
- R10: Stage-A exact-field and malformed-wire contracts SHALL be authored but not executed.

## Architecture

Wire decoding belongs beside existing MTProto service parsers in `native/telegram-protocol/src/api.rs`, re-exported by the protocol crate. `native/telegram-network/src/lib.rs` consumes the typed parser in the existing single-outstanding-RPC receive path, then preserves the validated raw object as the RPC result. Desktop receives byte-identical synchronized provider copies. No UI, storage, account, callback registry, request-history store, or second Telegram runtime is added.

## Failure handling

- Wrong top-level/vector/nested constructor -> protocol parse failure -> `NetworkError::InvalidServiceMessage` before settlement.
- Truncated/impossible-count/trailing object -> protocol parse failure -> `NetworkError::InvalidServiceMessage` before settlement.
- Valid unrelated `req_msg_id` -> continue waiting without current-call mutation.
- Valid matching `req_msg_id` -> return the already-validated raw object through the existing `RpcResult`.
- Historical multi-request correlation remains fail-closed/out of scope rather than being inferred.

## Verification strategy

Stage A only authors source-level contracts for:
- exact top-level and nested field parsing;
- wrong top-level/vector/nested constructor rejection;
- truncation, impossible vector count, and trailing-byte rejection;
- one-outstanding-RPC matching;
- typed-parser-before-settlement source ordering.

Do not run build, typecheck, lint, schema validation, generators, unit/contract/integration/E2E tests, packaging, deployment, installation, GitHub Actions, checks, or statuses during Stage A.

## Acceptance criteria

- AC1: Complete fixed-U `future_salts` wire decoding is owned by `telegram-protocol`.
- AC2: Declared vector length is structurally bounded by remaining bytes before allocation.
- AC3: The network receive branch performs typed parsing before request correlation or settlement.
- AC4: Current-request-only settlement remains bounded to the one outstanding RPC.
- AC5: Wrong/truncated/impossible/trailing objects fail closed.
- AC6: Desktop provider copies are byte-identical to canonical protocol/network sources.
- AC7: No gzip/difference/multi-request history scope is claimed.
- AC8: Authored tests remain not-run and acceptance remains false until Stage B executable evidence exists.


## Round 36 request-side extension

At live HEAD `5e4a142d6090bcfd233b9913c1dbb16caa999a7b`, the response-side `FutureSalt` / `FutureSalts` typed parser and current-request correlation already exist, but the independently representable fixed-U request `get_future_salts#b921bd04 num:int = FutureSalts` has no canonical exact serializer. The existing network/session owners do not precisely model when to schedule salt refreshes or how to install/rotate future salts across historical/session state, so this extension closes request bytes only and keeps scheduling/salt lifecycle fail-closed.

- R11: The canonical protocol owner SHALL export `GET_FUTURE_SALTS_CONSTRUCTOR = 0xb921bd04`.
- R12: The canonical protocol owner SHALL expose `build_get_future_salts(num: i32) -> Vec<u8>`.
- R13: The builder SHALL encode exactly the request constructor followed by `num:int`, little-endian, for exactly 8 bytes.
- R14: The builder SHALL preserve the complete signed `i32` domain without applying scheduling, range, retry, or salt-lifecycle policy.
- R15: The request builder and its contracts SHALL remain structurally separate from `parse_future_salts`, `FUTURE_SALTS_CONSTRUCTOR`, and the receive/current-request correlation path.
- R16: Authored-only request contracts SHALL cover exact layout, a representative value, `i32::MIN` / `i32::MAX`, and request/response-parser separation; they SHALL NOT be executed in Stage A.
- R17: Exact request serialization SHALL have no socket, session-salt, timer, retry, persistence, request-history, or response-settlement side effects. Live request scheduling and future-salt installation/rotation remain explicit fail-closed scope.

Additional acceptance criteria:

- AC9: Exact `get_future_salts` request serialization is owned by the existing source-neutral `telegram-protocol` owner.
- AC10: The serializer emits exactly 8 bytes and preserves signed `num:int` without policy transformation.
- AC11: Request serialization stays separate from `FutureSalts` response parsing/current-request correlation.
- AC12: Desktop protocol provider sources are synchronized byte-for-byte from canonical sources.
- AC13: Live salt-refresh scheduling/installation lifecycle remains unclaimed/fail-closed until a precise existing owner can represent it; authored request tests remain not-run and accepted=false.


## Stage-A spec compliance review — 2026-11-01

Implementation commit: `f631d7cc0c1a21db93ab647d3b6dcb29b42cb74e`  
Desktop synchronization commit: `a3329a8c948cbffab1d6843c78abc8024afa87b4`  
Canonical blobs: protocol api `bdac614fab9ca16ffeb36c7e98a012fe20231597`, protocol lib `6b6bde59a9f5e86ec33b0258e1e6a34f2b1622f9`, network `b2eb7022c6a8d38d7ac9f4a9fd13fe22913621de`.

- R1: passed by source inspection — the protocol owner exports both fixed-U constructors, typed `FutureSalt` / `FutureSalts`, `parse_future_salts`, and the public crate re-exports.
- R2: passed by source inspection — the parser consumes `req_msg_id`, `now`, the TL vector constructor/count, and every nested `future_salt` constructor plus all three fields.
- R3: passed by source inspection — wrong top-level/vector/nested constructors, field truncation, impossible declared count, and trailing bytes all return parse failure.
- R4: passed by source inspection — after reading the bounded vector count, `count * 20` is checked against remaining bytes before `Vec::with_capacity(count)`.
- R5: passed by source inspection — `EstablishedSession::scan_rpc_body` invokes `parse_future_salts(body)` before request matching or settlement.
- R6: passed by source inspection — `future_salts_matches_request` gates settlement to the one outstanding request id.
- R7: passed by source inspection — a valid unrelated request id returns `Continue` without retry/session mutation.
- R8: passed by source inspection — a matching object returns `body.to_vec()` only after successful typed validation.
- R9: passed by ownership/scope inspection — changes stay in existing protocol/network owners and byte-identical desktop provider copies; no gzip, full difference decoder, or historical sent/resend/container owner was added.
- R10: passed as an authored-only Stage-A artifact — exact/malformed/parser-correlation contracts were added but not run.

Acceptance criteria AC1–AC7 are satisfied at source/implementation level. AC8 remains intentionally **blocked/not-run** until Stage B executable validation. No build, typecheck, lint, schema validator, generator, unit/contract/integration/E2E, package, deployment, installation, GitHub Actions, checks, or statuses were run or queried in this review.


## Round 36 request-side source-level compliance readback

Implementation commit: `58a9635f0286283218f6d8b7b2b14dd466bd4b98`.  
Desktop provider synchronization commit: `dd232c52e1b39cdccb409c6d6307f1523e4d538f`.  
Desktop bookkeeping commit: `e91b2242931aa8f6b2a4cf6a88952283c914ac65`.  
Canonical and desktop protocol blobs: api `e8026c79b8b14261ef30d39bf6de8b0bc92306a6`, lib `809eeb7aa3256e1aa259e694f413b9c0377f08d6`.

- R11: satisfied by source inspection — `GET_FUTURE_SALTS_CONSTRUCTOR` is exported as fixed-U `0xb921bd04`.
- R12: satisfied by source inspection — `build_get_future_salts(num: i32) -> Vec<u8>` is public and re-exported by the protocol crate.
- R13: satisfied by source inspection — the builder writes only the constructor then one little-endian `i32`, yielding the exact 8-byte contract.
- R14: satisfied by source inspection — `num` is written directly with no validation, clamp, retry, scheduling, or salt-lifecycle transformation.
- R15: satisfied by source inspection — request serialization is a distinct builder; `parse_future_salts` and response/current-request correlation are unchanged and remain receive-side responsibilities.
- R16: satisfied as an authored-only Stage-A artifact — exact-layout, signed-boundary, and request/response-parser separation contracts exist but were not run.
- R17: satisfied by ownership/source inspection — the builder has no socket, timer, session-salt, persistence, retry, response-settlement, or request-history side effect; live scheduling/install/rotation remains unclaimed/fail-closed.

AC9–AC12 are satisfied at source/implementation/bookkeeping level. AC13 is satisfied for the fail-closed ownership boundary and remains **blocked/not-run** for executable validation: `validation_status=not-run`, `accepted=false`. No build, typecheck, lint, schema validator, generator, unit/contract/integration/E2E, package, deployment, installation, GitHub Actions, checks, or statuses were run or queried.
