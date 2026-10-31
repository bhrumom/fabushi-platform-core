# Telegram MTProto http_wait exact wire validation

Status: Stage A implementation contract; executable validation deferred.

## Context and scope

This spec closes only the fixed-U typed wire-decoding responsibility for `http_wait#9299359f max_delay:int wait_after:int max_wait:int = HttpWait` inside the existing canonical `telegram-protocol` owner. It intentionally separates the object's exact TL wire shape from Telegram Desktop's HTTP long-poll connection scheduling.

It does not add, change, or infer an HTTP transport lifecycle, `usingHttpWait()` policy, connection retry/backoff, request queue/history, resend/container bookkeeping, gzip handling, or updates difference decoding.

## Fixed-U provenance and verified current state

- Upstream: `telegramdesktop/tdesktop@f985723b552b30fe1c17d377d57f44e61a15a8c4`.
- `Telegram/SourceFiles/mtproto/scheme/mtproto.tl@76225965bfc794388db241f3f760db8ab79e84f8` declares:
  - `http_wait#9299359f max_delay:int wait_after:int max_wait:int = HttpWait`.
- `Telegram/SourceFiles/mtproto/session_private.cpp@13cbc76ec723386027b5b4dcac79999b791c4614` constructs `MTP_http_wait(MTP_int(100), MTP_int(30), MTP_int(25000))` only when `_connection->usingHttpWait()`; no independent receive-side `mtpc_http_wait` service-message branch was found in the fixed-U handler.
- At the live canonical checkpoint before this spec, `native/telegram-protocol/src/api.rs` has exact typed parsers for adjacent MTProto objects but no `HTTP_WAIT_CONSTRUCTOR`, typed `HttpWait`, or exact `parse_http_wait`.

## Goal

Provide one source-neutral, side-effect-free canonical parser that preserves all three fixed-U TL `int` fields exactly and fails closed on malformed wire input, without claiming ownership of the HTTP connection wait lifecycle.

## Non-goals

- Do not hard-code Telegram Desktop's observed `100/30/25000` values as protocol validity constraints.
- Do not create an HTTP polling scheduler, transport mode, second Telegram runtime/provider, request registry, callback registry, or connection-state machine.
- Do not add `wasSent/haveSent/_resendingIds/toSend/_sentContainers` semantics.
- Do not implement `gzip_packed` or complete updates difference/differenceSlice decoding/application.

## Requirements

- R1: The canonical protocol owner SHALL export fixed-U `HTTP_WAIT_CONSTRUCTOR = 0x9299359f`.
- R2: The canonical protocol owner SHALL expose a typed `HttpWait` value preserving `max_delay:i32`, `wait_after:i32`, and `max_wait:i32` in schema order.
- R3: `parse_http_wait` SHALL accept exactly one `http_wait` object: constructor followed by exactly three little-endian TL `int` values.
- R4: Wrong constructor, truncation at any field boundary or byte boundary, and any trailing bytes SHALL fail closed through the existing protocol error boundary.
- R5: The parser SHALL be side-effect free and SHALL NOT mutate transport/session/auth/request state.
- R6: HTTP connection wait scheduling remains explicitly outside this slice because the existing canonical owners do not yet expose a proven equivalent of Telegram Desktop's `usingHttpWait()` lifecycle.
- R7: Desktop provider protocol sources SHALL be synchronized byte-for-byte only after the canonical implementation is durably committed.
- R8: Stage A SHALL only author tests/contracts; it SHALL NOT run build, typecheck, lint, schema validation, generators, tests, package, deploy, install, Actions, checks, or statuses.

## Interface and failure contract

Canonical API:

- `HttpWait { max_delay, wait_after, max_wait }`
- `parse_http_wait(&[u8]) -> Result<HttpWait, ApiRequestError>`

Failures:

- constructor != `0x9299359f` -> existing unexpected-constructor TL error;
- insufficient bytes for any `int` -> existing TL truncation error;
- bytes remaining after the third `int` -> `ApiRequestError::TrailingBytes`;
- decoded values have no transport/session side effect by themselves.

## Architecture and ownership

- Wire schema decoding owner: existing `bhrumom/fabushi-platform-core:native/telegram-protocol`.
- Desktop consumer copy: existing `source/mahayana/mahayana-rs/providers/telegram-protocol`.
- HTTP wait connection lifecycle: not introduced here; remains fail-closed/open until an existing owner can precisely represent the fixed-U scheduling semantics.
- No new owner, runtime, provider, storage, or UI root is introduced.

## Implementation strategy

1. Add the fixed-U constructor constant, typed value, and exact parser to `native/telegram-protocol/src/api.rs`.
2. Re-export them from `native/telegram-protocol/src/lib.rs`.
3. Author parser contracts for exact field preservation, wrong constructor, truncation, and trailing bytes; do not execute them.
4. Durably commit canonical implementation.
5. Synchronize the canonical protocol files byte-for-byte into the existing desktop provider.
6. Update the existing TDRP ledger/P0/source-closure/evidence with `implemented / validation_status=not-run / accepted=false`.

## Verification strategy

Stage A source-level review will verify:

- constructor and field order match fixed-U `mtproto.tl`;
- all three `i32` values are preserved exactly;
- malformed/trailing input fails closed;
- no HTTP scheduling or request-history mutation is introduced;
- desktop provider blobs are byte-identical to canonical blobs after synchronization;
- authored tests remain unexecuted.

No executable validation is permitted in this stage.

## Acceptance criteria

- AC1: `HTTP_WAIT_CONSTRUCTOR`, `HttpWait`, and `parse_http_wait` exist in the canonical protocol owner.
- AC2: Exact fixed-U body decoding preserves `max_delay`, `wait_after`, and `max_wait`.
- AC3: Wrong constructor, truncation, and trailing bytes fail closed.
- AC4: No connection-wait lifecycle or historical request/container semantics are invented.
- AC5: Desktop provider api/lib blobs are byte-identical to the canonical implementation after synchronization.
- AC6: TDRP bookkeeping records the responsibility as implemented while validation remains not-run and accepted remains false.
- AC7: Stage-B executable evidence remains pending.

## References

- `telegramdesktop/tdesktop@f985723b552b30fe1c17d377d57f44e61a15a8c4`
- fixed-U `mtproto.tl@76225965bfc794388db241f3f760db8ab79e84f8`
- fixed-U `session_private.cpp@13cbc76ec723386027b5b4dcac79999b791c4614`
- `docs/specs/spec-first-ai-development.md`


## Source-level implementation review

Canonical implementation commit: `96bb0e2cd31cdfc82aaa44e0f8e25acf8c30a204`. Canonical blobs: protocol api `7b731b6d6bbd2009453c5107a24ebea58cc2ece3`, protocol lib `4d8b7198fbe7aa8cbb71180587970760f6ebd1d9`. Desktop synchronization commit: `1495975bbdd2e5d56826a95854def198862e2c5b`; desktop bookkeeping/evidence commit: `60fa3f0ad601bcffaa8acdc6e0f26a7a52bea24e`.

- R1-R4: satisfied by source inspection — the protocol owner exports `HTTP_WAIT_CONSTRUCTOR`, typed `HttpWait`, and `parse_http_wait`; the parser requires the fixed-U constructor, consumes exactly three signed TL ints in schema order, preserves arbitrary i32 values, and rejects wrong constructors, truncation, and trailing bytes.
- R5-R6: satisfied by source inspection and repository state — the parser is side-effect free; no HTTP scheduler, `usingHttpWait()` lifecycle, session/request mutation, or historical sent/container bookkeeping was added. Connection-wait scheduling remains an explicit blocker instead of being inferred.
- R7: satisfied by remote blob readback — desktop provider api/lib blobs are byte-identical to canonical api/lib blobs.
- R8: satisfied for Stage A process — contracts were authored only; no build, typecheck, lint, schema validator, generator, unit/contract/integration/E2E, package, deployment, installation, GitHub Actions, checks, or statuses were run or queried.

Acceptance criteria AC1-AC6 are satisfied at source/implementation/bookkeeping level. AC7 remains intentionally **blocked/not-run** until Stage B executable validation. No runtime claim is made for the HTTP connection-wait lifecycle.


## Round 64 adjacent revalidation — fixed-U http_wait exact wire

At live checkpoint `1542bb5e40d2b3c9d4b21ac59f99cb420d7fdbb7`, fixed U remains `telegramdesktop/tdesktop@f985723b552b30fe1c17d377d57f44e61a15a8c4`. Rereading `mtproto.tl@76225965bfc794388db241f3f760db8ab79e84f8` and `session_private.cpp@13cbc76ec723386027b5b4dcac79999b791c4614` confirms `http_wait#9299359f max_delay:int wait_after:int max_wait:int = HttpWait` is shipping-reachable only as the object serialized by `SessionPrivate::tryToSend()` when `_connection->usingHttpWait()`; there is no independent receive-side `mtpc_http_wait` branch. The observed `100 / 30 / 25000` call-site values remain lifecycle evidence, not schema validity constraints.

This is a source-level **no-production-delta** revalidation. Current canonical `native/telegram-protocol/src/api.rs@ebfb3670b5cd097b9ce288782c7954f5cfe9fc43` still exports `HTTP_WAIT_CONSTRUCTOR`, typed `HttpWait { max_delay:i32, wait_after:i32, max_wait:i32 }`, and `parse_http_wait`; `native/telegram-protocol/src/tl.rs@89256ab35184b6dc06a095ab34abb9f44e95a264` supplies bounded little-endian signed `i32` reads, while `native/telegram-protocol/src/lib.rs@abab39e42f100972484191f0e52e9edd9b88c6cf` re-exports the same owner. The parser requires constructor + three fields = exactly 16 bytes, rejects constructor mismatch and every truncation through the bounded reader, and rejects any trailing bytes.

Round-64 authored-only external contracts SHALL pin exact length 16, offsets `0..4 / 4..8 / 8..12 / 12..16`, independent signed `i32::MIN / -1 / i32::MAX` preservation across the three fields, every truncation cut `0..15`, wrong-constructor rejection, strict EOF, and shall not hard-code the fixed-U call-site values as validity constraints. These contracts SHALL NOT be executed during Stage A.

The wire-only parser remains intentionally separate from `usingHttpWait()` transport selection/scheduling, timeout/retry/backoff, request queues/history, `wasSent/haveSent/_resendingIds/toSend/_sentContainers`, auth-key binding/persistence, `gzip_packed`, and complete difference/differenceSlice decoding/application. The existing ledger responsibility `TDRP-R9-MTPROTO-HTTP-WAIT-WIRE-001` remains the sole row and stays `implemented / validation_status=not-run / accepted=false`; coverage remains `responsibilities_total=97, unknown=16172, unread=16145, omitted=0`.

No GitHub Actions/status/check query or trigger, build, typecheck, lint, schema validator, generator, unit/contract/integration/E2E test, package, deployment, installation, or runtime acceptance was performed.
