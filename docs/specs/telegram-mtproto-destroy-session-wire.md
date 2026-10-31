# Telegram MTProto DestroySessionRes exact wire validation

Status: Stage A implementation contract; executable validation deferred.

## Scope

This spec closes only the fixed-U typed wire-decoding responsibility for `destroy_session_ok` and `destroy_session_none` inside the existing canonical `telegram-protocol` owner. It does not add an outbound `destroy_session` production lifecycle, session teardown orchestration, historical request correlation, sent/resend/container bookkeeping, gzip handling, or updates difference decoding.

## Fixed-U provenance

- Upstream: `telegramdesktop/tdesktop@f985723b552b30fe1c17d377d57f44e61a15a8c4`.
- `Telegram/SourceFiles/mtproto/scheme/mtproto.tl@76225965bfc794388db241f3f760db8ab79e84f8`
  - `destroy_session_ok#e22045fc session_id:long = DestroySessionRes`.
  - `destroy_session_none#62d350c9 session_id:long = DestroySessionRes`.
  - `destroy_session#e7512126 session_id:long = DestroySessionRes`.
- `Telegram/SourceFiles/mtproto/session_private.cpp@13cbc76ec723386027b5b4dcac79999b791c4614` has no direct `destroy_session_*` receive branch in the fixed-U session service-message handler; these constructors are schema-defined RPC result bodies rather than an independently handled session service-message branch.

## Pre-implementation gap

At the live checkpoint before this spec, `native/telegram-protocol/src/api.rs` has typed exact parsers for adjacent MTProto objects including `rpc_result`, `RpcDropAnswer`, `pong`, `future_salts`, and detailed-info objects, but it has no constants, typed value, or exact parser for `DestroySessionRes`. The current canonical/desktop production composition also has no established outbound `destroy_session` command/call site whose teardown semantics can be safely inferred here.

## Requirements

- R1: The canonical protocol owner SHALL export both fixed-U result constructor IDs and a typed `DestroySessionResult` value with Ok and None variants that preserve `session_id:long`.
- R2: `destroy_session_ok#e22045fc` SHALL decode exactly one `session_id:long`.
- R3: `destroy_session_none#62d350c9` SHALL decode exactly one `session_id:long`.
- R4: Unknown constructor, truncation of either result body, and trailing bytes SHALL fail closed through the existing protocol error boundary.
- R5: This slice SHALL NOT mutate current session/auth/request state, synthesize teardown behavior, or introduce `wasSent/haveSent/_resendingIds/toSend/_sentContainers` history.
- R6: The absence of a canonical outbound destroy-session + teardown lifecycle SHALL remain an explicit blocker for interpreting either result as a live session mutation; only exact wire decoding is implemented here.
- R7: Desktop provider protocol sources SHALL be synchronized byte-for-byte only after the canonical implementation is durably committed.

## Failure contract

- Unknown constructor -> protocol parse failure.
- Missing or truncated `session_id` -> protocol parse failure.
- Any trailing bytes -> `ApiRequestError::TrailingBytes`.
- A decoded result causes no transport/session side effect by itself.
- No live session is removed, retired, or mutated in this slice.

## Verification strategy

Stage A only authors source-level contracts for:
- exact decoding and session-id preservation for both fixed-U result variants;
- unknown-constructor, truncation, and trailing-byte rejection;
- byte-identical desktop provider synchronization;
- explicit absence of teardown/history mutation.

Do not run build, typecheck, lint, schema validation, generators, unit/contract/integration/E2E tests, packaging, deployment, installation, GitHub Actions, checks, or statuses during Stage A.

## Acceptance criteria

- AC1: Both fixed-U `DestroySessionRes` constructors have one canonical typed parser in `telegram-protocol`.
- AC2: Ok and None each preserve exactly one `session_id:long`.
- AC3: Unknown constructors, truncation, and trailing bytes fail closed.
- AC4: No session teardown/history semantics or second owner is introduced.
- AC5: Desktop provider protocol sources are byte-identical to canonical protocol sources after synchronization.
- AC6: Authored tests remain not-run and acceptance remains false until Stage B executable evidence exists.
