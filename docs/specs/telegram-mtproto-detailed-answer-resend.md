# Telegram MTProto detailed-answer resend — Specification

Status: active  
Owner: shared Telegram protocol/network runtime  
Last updated: 2026-11-01  
Related task: Fabushi migration task 7457eb1c-11a3-4626-8563-a3e495872536 round 26

## Context

Fixed upstream `telegramdesktop/tdesktop@f985723b552b30fe1c17d377d57f44e61a15a8c4` handles `msg_detailed_info#276d3ec6` by fully decoding `msg_id`, `answer_msg_id`, `bytes`, and `status`, acknowledging the referenced sent request, and requesting retransmission of `answer_msg_id` when that answer has not been received.

Fabushi already has the bounded one-outstanding-RPC resend behavior in the canonical `EstablishedSession::invoke_raw` path, but the current receive branch reads only `msg_id` and `answer_msg_id` from raw offsets. It therefore accepts a payload that is long enough for those two fields even when `bytes/status` are truncated, and it ignores trailing bytes. That partial read is not an exact fixed-U wire contract and can produce ACK/resend side effects before the whole service message is validated.

## Fixed-U provenance

- `Telegram/SourceFiles/mtproto/session_private.cpp@13cbc76ec723386027b5b4dcac79999b791c4614`
  - `mtpc_msg_detailed_info` uses generated TL decoding before reading all four fields;
  - the sent request is considered acknowledged;
  - a received `answer_msg_id` is ACKed and a missing answer is queued for `msg_resend_req`.
- `Telegram/SourceFiles/mtproto/scheme/mtproto.tl@76225965bfc794388db241f3f760db8ab79e84f8`
  - `msg_detailed_info#276d3ec6 msg_id:long answer_msg_id:long bytes:int status:int`;
  - `msg_resend_req#7d861a08 msg_ids:Vector<long>`.

## Requirements

- R1: The canonical protocol owner SHALL export `MSG_DETAILED_INFO_CONSTRUCTOR`, a typed `MsgDetailedInfo`, and `parse_msg_detailed_info(&[u8])`.
- R2: The parser SHALL decode exactly `msg_id:long`, `answer_msg_id:long`, `bytes:int`, and `status:int`, and SHALL reject a wrong constructor, truncation of any field, and trailing bytes.
- R3: The canonical protocol owner SHALL continue to expose typed `build_msg_resend_req(&[i64])` using the fixed-U constructor.
- R4: `EstablishedSession::scan_rpc_body` SHALL parse the complete `msg_detailed_info` object before any acknowledgement or resend side effect.
- R5: A valid detailed-info object SHALL affect the current call only when its `msg_id` equals the one outstanding request.
- R6: For a matching request, the existing session SHALL reuse `reconcile_detailed_answer(answer_msg_id, ...)`: an already-received answer is accumulated for ACK at most once; a missing answer sends one non-content-related `msg_resend_req`; the RPC remains outstanding.
- R7: A valid unrelated `msg_detailed_info.msg_id` SHALL be ignored for the current call and SHALL NOT mutate retry state.
- R8: Parsing, matching, replay-window ownership, service-message sending, auth key, session id, transport, sequence, and message-id guards SHALL remain in the existing protocol/network owners; no second request history/runtime/provider is permitted.
- R9: This slice SHALL NOT implement `gzip_packed`, complete update-difference decoding, or Telegram Desktop's `wasSent/haveSent/_resendingIds/toSend/_sentContainers` multi-request history.
- R10: Stage-A exact-field and malformed-wire contracts SHALL be authored but not executed.

## Architecture

Wire decoding belongs beside the existing MTProto service parsers in `native/telegram-protocol/src/api.rs` and is re-exported by that crate. The network owner consumes the typed parser in the existing `EstablishedSession::scan_rpc_body` branch, then reuses the already-existing `detailed_info_matches_request` and `reconcile_detailed_answer` lifecycle. Desktop consumes byte-identical synchronized provider copies. No UI, storage, account, or second Telegram owner is added.

## Failure handling

- Wrong constructor, truncated body, or trailing data -> protocol parse failure -> `NetworkError::InvalidServiceMessage` before ACK/resend.
- Valid but unrelated `msg_id` -> continue without current-call mutation.
- Valid matching `msg_id` -> existing bounded answer reconciliation.
- Missing historical multi-request state -> remain fail-closed/out of scope rather than inventing queue semantics.

## Verification strategy

Stage A only authors source-level contracts for:
- exact four-field parsing;
- wrong-constructor rejection;
- truncation rejection;
- trailing-byte rejection;
- current-request-only matching;
- existing same-session ACK/resend behavior.

Do not run build, typecheck, lint, schema validation, generators, unit/contract/integration/E2E tests, packaging, deployment, installation, GitHub Actions, checks, or statuses during Stage A.

## Acceptance criteria

- AC1: Complete fixed-U `msg_detailed_info` wire decoding is owned by `telegram-protocol`.
- AC2: The network receive branch performs typed parsing before any side effect.
- AC3: Current-request matching and existing answer reconciliation remain bounded to the one outstanding RPC.
- AC4: Wrong/truncated/trailing objects fail closed.
- AC5: Desktop provider copies are byte-identical to canonical protocol/network sources.
- AC6: No gzip/difference/multi-request history scope is claimed.
- AC7: Authored tests remain not-run and acceptance remains false until Stage B executable evidence exists.


## Stage-A spec compliance review — 2026-11-01

Implementation commit: `4c5bea7b847a3e04ee3d0c9b05fab228101acb64`  
Canonical blobs: protocol api `5da871fc557a49a7e57f1dd0677f307a5ed1bac8`, protocol lib `3f7c363f9fd2eb86e60c8294144d7f1dced9ccca`, network `2a50068d148b25e710961973315d78fc1b1ee5a8`.

- R1: passed by source inspection — constructor, typed struct, parser, and public re-export are present in the canonical protocol owner.
- R2: passed by source inspection — the parser consumes all four fixed-U fields and rejects wrong constructor, truncation through `TlReader`, and trailing bytes.
- R3: passed by source inspection — the existing typed `build_msg_resend_req` remains the service primitive.
- R4: passed by source inspection — `scan_rpc_body` invokes `parse_msg_detailed_info` before matching or reconciliation.
- R5: passed by source inspection — the existing `detailed_info_matches_request` gates current-call effects to the single outstanding request id.
- R6: passed by source inspection — matching messages reuse `reconcile_detailed_answer`; no new queue/state owner was added.
- R7: passed by source inspection — unrelated valid messages return `Continue` without current-call mutation.
- R8: passed by source inspection — changes stay within existing protocol/network owners and synchronized desktop provider copies.
- R9: passed by scope inspection — no gzip, full difference decoder, or multi-request sent/resend history was introduced.
- R10: passed as an authored-only Stage-A artifact — exact-field, wrong-constructor, truncation, and trailing-byte tests were added to source but not run.

Acceptance criteria AC1–AC6 are satisfied at the source/implementation level. AC7 remains intentionally **blocked/not-run** until Stage B executable validation. No build, typecheck, lint, schema validator, generator, unit/contract/integration/E2E, package, deployment, installation, GitHub Actions, checks, or statuses were run or queried in this review.
