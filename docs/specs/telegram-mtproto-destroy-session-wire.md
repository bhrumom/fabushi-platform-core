# Telegram MTProto destroy_session request and DestroySessionRes exact wire contracts

Status: Stage A implementation contract; executable validation deferred.

## Scope

This spec closes the fixed-U exact wire responsibilities for the `destroy_session#e7512126` request serializer and the already-implemented typed decoding of `destroy_session_ok` / `destroy_session_none` inside the existing canonical `telegram-protocol` owner. Request serialization and result parsing are deliberately separate contracts. This spec does not add an outbound live teardown production lifecycle, session teardown orchestration, historical request correlation, sent/resend/container bookkeeping, gzip handling, or updates difference decoding.

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

## Round 35 request-side gap and requirements

At live HEAD `6cfc81d79901493c6b34ec4e174d4fbdae81800e`, the response-side constants, typed `DestroySessionResult`, and strict `parse_destroy_session_result` already exist, but the request-side constructor `destroy_session#e7512126` and exact serializer are still absent. The current repository still has no precise live teardown/outbound-session lifecycle owner. The latest user requirement therefore extends this durable spec only with the independently representable request bytes.

- R8: The canonical protocol owner SHALL export `DESTROY_SESSION_CONSTRUCTOR = 0xe7512126`.
- R9: The canonical protocol owner SHALL expose `build_destroy_session(session_id: i64) -> Vec<u8>`.
- R10: The builder SHALL encode exactly request constructor then `session_id:long`, little-endian, for exactly 12 bytes.
- R11: The builder SHALL preserve the complete signed `i64` domain without lifecycle/policy validation.
- R12: The builder and its contracts SHALL remain structurally separate from `parse_destroy_session_result` and the result constructors.
- R13: Authored-only request contracts SHALL cover exact layout, representative value, and `i64::MIN` / `i64::MAX`; they SHALL NOT be executed in Stage A.
- R14: Exact request serialization SHALL have no socket/session/timer/history/persistence side effects. Live teardown/outbound-session lifecycle remains explicit fail-closed/open.

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


## Round 35 request-side acceptance criteria

- AC7: canonical `DESTROY_SESSION_CONSTRUCTOR` and `build_destroy_session` exist.
- AC8: source inspection shows exactly constructor + one `i64` and no policy transformation or extra bytes.
- AC9: request builder/contracts remain separate from the existing `DestroySessionRes` parser; desktop provider api/lib are byte-identical after synchronization.
- AC10: request contracts are authored but not run; ledger/P0/source-closure/evidence record `implemented / validation_status=not-run / accepted=false`; live teardown remains fail-closed.

## Source-level implementation review

Canonical implementation commit: `cfe6a0564c40d2c9b9ea62b686470139a1eeb2ab`. Canonical blobs: protocol api `d82cca134274b8d939c1140ec44ee00230e2cfac`, protocol lib `655498ce9db2faf8a76f093250d4a1f02f77ff67`. Desktop synchronization commit: `b179796344fba3e22bccb1310b74f229d613b531`; desktop bookkeeping/evidence commit: `c4329d6c1212d1f6309c0876580e5fe5a10b4ca7`.

- R1-R4: satisfied by source inspection — the protocol owner exports both fixed-U result constructor IDs, typed `DestroySessionResult`, and `parse_destroy_session_result`; Ok and None each consume exactly one `session_id:long`; unknown constructors, truncation, and trailing bytes fail closed.
- R5-R6: satisfied by source inspection and repository state — no outbound `destroy_session` production command/call path, live teardown mutation, historical request/container mutation, or second owner was added. The missing canonical teardown lifecycle remains an explicit blocker rather than being synthesized in telegram-network.
- R7: satisfied by remote blob readback — desktop provider api/lib blobs are byte-identical to canonical api/lib blobs.

Acceptance criteria AC1-AC5 are satisfied at source/implementation level. AC6 remains intentionally **blocked/not-run** until Stage B executable validation. No build, typecheck, lint, schema validator, generator, unit/contract/integration/E2E, package, deployment, installation, GitHub Actions, checks, or statuses were run or queried in this review.
