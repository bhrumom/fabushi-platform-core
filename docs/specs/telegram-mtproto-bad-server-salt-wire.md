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
