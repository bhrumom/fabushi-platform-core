# Telegram MTProto bad-server-salt wire validation — Specification

Status: active  
Owner: shared Telegram protocol/network runtime  
Last updated: 2026-11-01  
Related task: Fabushi migration task 7457eb1c-11a3-4626-8563-a3e495872536 round 28

## Context

Fixed upstream `telegramdesktop/tdesktop@f985723b552b30fe1c17d377d57f44e61a15a8c4` declares `bad_server_salt#edab447b` as `bad_msg_id:long bad_msg_seqno:int error_code:int new_server_salt:long`.

Fabushi's canonical `EstablishedSession` currently recognizes constructor `edab447b` in both the RPC and ping/service receive paths, but reads only raw offsets for `bad_msg_id`, `error_code`, and `new_server_salt`. It neither decodes `bad_msg_seqno` nor proves that the object ends exactly after `new_server_salt`. A body with a matching request id but malformed/truncated sequence field placement or trailing bytes can therefore mutate the current server salt without complete fixed-U wire validation.

## Fixed-U provenance

- `Telegram/SourceFiles/mtproto/scheme/mtproto.tl@76225965bfc794388db241f3f760db8ab79e84f8`
  - `bad_msg_notification#a7eff811 bad_msg_id:long bad_msg_seqno:int error_code:int = BadMsgNotification`;
  - `bad_server_salt#edab447b bad_msg_id:long bad_msg_seqno:int error_code:int new_server_salt:long = BadMsgNotification`.
- `Telegram/SourceFiles/mtproto/session_private.cpp@13cbc76ec723386027b5b4dcac79999b791c4614`
  - the `mtpc_bad_server_salt` branch first decodes a generated TL object, then reads `vbad_msg_id`, `vbad_msg_seqno`, `verror_code`, and `vnew_server_salt` before applying salt/retry behavior.

## Requirements

- R1: The canonical protocol owner SHALL export `BAD_SERVER_SALT_CONSTRUCTOR`, typed `BadServerSalt`, and `parse_bad_server_salt(&[u8])`.
- R2: The parser SHALL decode exactly `bad_msg_id:long`, `bad_msg_seqno:int`, `error_code:int`, and `new_server_salt:long`.
- R3: Wrong constructor, truncation at any field boundary or within any field, and any trailing bytes SHALL fail closed.
- R4: `EstablishedSession::scan_rpc_body` SHALL invoke the typed parser before request matching, server-salt mutation, retry signaling, or settlement.
- R5: The ping/service receive path SHALL invoke the same typed parser before request matching or server-salt mutation.
- R6: A valid `bad_server_salt.bad_msg_id` SHALL affect only the one currently outstanding request when it exactly matches that request id. A valid unrelated id SHALL preserve the existing fail-closed `BadServerMessage` behavior.
- R7: The parsed `bad_msg_seqno` SHALL be retained in the typed protocol value even though this bounded single-outstanding-RPC slice does not introduce historical sent/container sequence reconciliation.
- R8: Existing bounded new-salt retry semantics and the exact `new_server_salt` value SHALL remain unchanged after successful validation.
- R9: Protocol parsing, authenticated transport, request correlation, and salt mutation SHALL remain in the existing telegram-protocol/telegram-network owners. This slice SHALL NOT introduce gzip decompression, complete updates-difference decoding, or `wasSent/haveSent/_resendingIds/toSend/_sentContainers` history.
- R10: Stage-A exact-field and malformed-wire contracts SHALL be authored but not executed.

## Architecture

Wire decoding belongs beside the existing MTProto service parsers in `native/telegram-protocol/src/api.rs`, re-exported by the protocol crate. `native/telegram-network/src/lib.rs` consumes the typed parser in both existing `bad_server_salt` receive branches and applies the existing single-request salt/retry behavior only after successful parsing. Desktop receives byte-identical synchronized provider copies. No second runtime, request-history store, UI state, storage owner, or callback registry is added.

## Failure handling

- Wrong constructor, truncation, or trailing object data -> protocol parse failure -> `NetworkError::InvalidServiceMessage` before salt mutation.
- Valid unrelated `bad_msg_id` -> existing `NetworkError::BadServerMessage` with parsed id/code.
- Valid matching object -> apply the parsed `new_server_salt` and existing bounded retry/new-salt outcome.
- The parsed sequence number is not used to infer historical request state in this slice.

## Verification strategy

Stage A only authors source-level contracts for:
- exact decoding of all four fixed-U fields;
- wrong-constructor rejection;
- truncation at multiple offsets and trailing-byte rejection;
- current-request-only correlation behavior;
- typed-parser-before-salt-mutation source ordering.

Do not run build, typecheck, lint, schema validation, generators, unit/contract/integration/E2E tests, packaging, deployment, installation, GitHub Actions, checks, or statuses during Stage A.

## Acceptance criteria

- AC1: Complete fixed-U `bad_server_salt` wire decoding is owned by telegram-protocol.
- AC2: Both network receive branches use typed parsing before request correlation or server-salt mutation.
- AC3: `bad_msg_seqno` is decoded and retained; trailing bytes fail closed.
- AC4: Current-request-only salt mutation/retry semantics remain bounded to the one outstanding request.
- AC5: Desktop provider copies are byte-identical to canonical protocol/network sources.
- AC6: No gzip/difference/multi-request history scope is claimed.
- AC7: Authored tests remain not-run and acceptance remains false until Stage B executable evidence exists.


## Stage-A spec compliance review — 2026-11-01

Implementation commit: `3a850171c59a5f27809e176ba54971c7c0c003dc`  
Canonical blobs: protocol api `75959eb82460cb36bf5f43c6e4470ef8814ea2a9`, protocol lib `3540f1a60df0d07f77b6e38349e0c500ec2d12d8`, network `42be9e9fa78c3acc77367799c1ecb506cdd25ab0`.

- R1: passed by source inspection — the protocol owner exports `BAD_SERVER_SALT_CONSTRUCTOR`, typed `BadServerSalt`, `parse_bad_server_salt`, and the public crate re-exports.
- R2: passed by source inspection — the parser consumes `bad_msg_id`, `bad_msg_seqno`, `error_code`, and `new_server_salt` in fixed-U order.
- R3: passed by source inspection — wrong constructor and any truncated read return parse failure; a non-finished reader returns `TrailingBytes`.
- R4: passed by source inspection — `EstablishedSession::scan_rpc_body` invokes `parse_bad_server_salt(body)` before current-request matching and before assigning `self.server_salt`.
- R5: passed by source inspection — the ping/service branch invokes the same typed parser before matching and salt mutation.
- R6: passed by source inspection — `bad_server_salt_matches_request` gates both branches to the one outstanding request; a valid unrelated id preserves `BadServerMessage` with the parsed id/code.
- R7: passed by source inspection — `BadServerSalt.bad_message_sequence_number` retains the decoded fixed-U sequence field without creating historical sent/container state.
- R8: passed by source inspection — a matching object assigns exactly `notification.new_server_salt` and returns the existing `NewSalt` outcome.
- R9: passed by ownership/scope inspection — changes stay in existing protocol/network owners; no gzip, full difference decoder, or historical `wasSent/haveSent/_resendingIds/toSend/_sentContainers` owner was introduced.
- R10: passed as an authored-only Stage-A artifact — exact-field, wrong-constructor, truncation, trailing-byte, and current-request contracts were added but not run.

Acceptance criteria AC1-AC4 and AC6 are satisfied at source/implementation level. AC5 is pending desktop synchronization in this round. AC7 remains intentionally **blocked/not-run** until Stage B executable validation. No build, typecheck, lint, schema validator, generator, unit/contract/integration/E2E, package, deployment, installation, GitHub Actions, checks, or statuses were run or queried in this review.
