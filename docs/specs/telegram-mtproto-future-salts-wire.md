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
