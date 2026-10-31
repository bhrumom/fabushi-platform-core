# Telegram MTProto RpcDropAnswer exact wire validation

Status: Stage A implementation contract; executable validation deferred.

## Scope

This spec closes only the fixed-U typed wire-decoding responsibility for the three `RpcDropAnswer` result objects inside the existing canonical `telegram-protocol` owner. It does not add an outbound `rpc_drop_answer` lifecycle, a historical sent-request store, callback registry, resend/container bookkeeping, gzip handling, or updates difference decoding.

## Fixed-U provenance

- Upstream: `telegramdesktop/tdesktop@f985723b552b30fe1c17d377d57f44e61a15a8c4`.
- `Telegram/SourceFiles/mtproto/scheme/mtproto.tl@76225965bfc794388db241f3f760db8ab79e84f8`
  - `rpc_answer_unknown#5e2ad36e = RpcDropAnswer`.
  - `rpc_answer_dropped_running#cd78e586 = RpcDropAnswer`.
  - `rpc_answer_dropped#a43ad8b7 msg_id:long seq_no:int bytes:int = RpcDropAnswer`.
  - `rpc_drop_answer#58e4a740 req_msg_id:long = RpcDropAnswer`.
- `Telegram/SourceFiles/mtproto/session_private.cpp@13cbc76ec723386027b5b4dcac79999b791c4614` has no direct `rpc_answer_*` branch; these constructors are schema-defined RPC result bodies rather than an independently handled session service-message branch.

## Pre-implementation gap

At the live checkpoint before this spec, `native/telegram-protocol/src/api.rs` has typed exact parsers for adjacent MTProto objects such as `rpc_result`, `msg_detailed_info`, `bad_msg_notification`, `bad_server_salt`, and `future_salts`, but it has no constants, typed value, or exact parser for the three `RpcDropAnswer` constructors. Neither canonical platform-core nor desktop has an outbound `rpc_drop_answer` command/call site.

## Requirements

- R1: The canonical protocol owner SHALL export the three fixed-U constructor IDs and a typed `RpcDropAnswer` value covering Unknown, DroppedRunning, and Dropped.
- R2: `rpc_answer_unknown#5e2ad36e` SHALL decode as a zero-field variant and reject any trailing bytes.
- R3: `rpc_answer_dropped_running#cd78e586` SHALL decode as a zero-field variant and reject any trailing bytes.
- R4: `rpc_answer_dropped#a43ad8b7` SHALL decode exactly `msg_id:long`, `seq_no:int`, and `bytes:int`, preserving all three values.
- R5: Unknown constructor, truncation at or within any Dropped field, and trailing bytes on every variant SHALL fail closed through the existing protocol error boundary.
- R6: This slice SHALL NOT add network/session mutation, request cancellation, historical request correlation, `wasSent/haveSent/_resendingIds/toSend/_sentContainers`, or an outbound `rpc_drop_answer` builder/call path.
- R7: The absence of a canonical historical request/drop lifecycle SHALL remain an explicit blocker for interpreting these results as request-history mutations; only exact wire decoding is implemented here.
- R8: Desktop provider copies SHALL be synchronized byte-for-byte only after the canonical implementation is durably committed.

## Failure contract

- Unknown constructor -> protocol parse failure.
- Truncated `rpc_answer_dropped` -> protocol parse failure.
- Any trailing bytes on any variant -> `ApiRequestError::TrailingBytes`.
- A decoded value causes no transport/session side effect by itself.
- No historical request is marked dropped/running/unknown in this slice.

## Verification strategy

Stage A only authors source-level contracts for:
- exact decoding of all three variants;
- exact preservation of `msg_id`, `seq_no`, and `bytes`;
- unknown-constructor, truncation, and trailing-byte rejection;
- byte-identical desktop provider synchronization.

Do not run build, typecheck, lint, schema validation, generators, unit/contract/integration/E2E tests, packaging, deployment, installation, GitHub Actions, checks, or statuses during Stage A.

## Acceptance criteria

- AC1: All three fixed-U `RpcDropAnswer` result constructors have one canonical typed parser in `telegram-protocol`.
- AC2: Both zero-field variants reject trailing bytes.
- AC3: The Dropped variant validates and preserves all three fixed-U fields and rejects truncation/trailing bytes.
- AC4: No historical request/container semantics or second owner is introduced.
- AC5: Desktop provider protocol sources are byte-identical to canonical protocol sources after synchronization.
- AC6: Authored tests remain not-run and acceptance remains false until Stage B executable evidence exists.

## Source-level implementation review

Canonical implementation commit: `5251ab3eacd8035cb8bb86609d76641233dd22eb`. Canonical blobs: protocol api `77b9cc0a7d191ef9f197fb8fd718e29631197ddc`, protocol lib `9229c83fa6b484a5af657ee37f0bd827fecdc28d`. Desktop synchronization commit: `4257fc9c875526c4b49a1f3cfbbeec7cd5a3eb0f`; desktop bookkeeping/evidence commit: `28799f961c18d95e1b5786dce23c312390a535b6`.

- R1-R5: satisfied by source inspection — the protocol owner exports all three fixed-U constructor IDs, typed `RpcDropAnswer`, and `parse_rpc_drop_answer`; Unknown and DroppedRunning are exact zero-field variants; Dropped consumes `msg_id`, `seq_no`, and `bytes`; unknown constructors, truncation, and trailing bytes fail closed.
- R6-R7: satisfied by source inspection and repository search — no outbound `rpc_drop_answer` builder/call site or historical request/drop mutation was added. The missing canonical historical request lifecycle remains an explicit blocker rather than being synthesized in telegram-network.
- R8: satisfied by remote blob readback — desktop provider api/lib blobs are byte-identical to canonical api/lib blobs.

Acceptance criteria AC1-AC5 are satisfied at source/implementation level. AC6 remains intentionally **blocked/not-run** until Stage B executable validation. No build, typecheck, lint, schema validator, generator, unit/contract/integration/E2E, package, deployment, installation, GitHub Actions, checks, or statuses were run or queried in this review.


## Round 38 request-side extension: rpc_drop_answer exact serializer

The latest explicit migration requirement extends this existing wire contract to the request side while preserving the parser/lifecycle boundary. The existing canonical `telegram-protocol` owner can represent the request bytes independently because fixed-U `rpc_drop_answer#58e4a740 req_msg_id:long = RpcDropAnswer` is a pure TL object with no inherent scheduling or request-history mutation.

Additional requirements:

- R9: The canonical protocol owner SHALL export fixed-U `RPC_DROP_ANSWER_CONSTRUCTOR = 0x58e4a740`.
- R10: The canonical protocol owner SHALL expose `build_rpc_drop_answer(req_msg_id: i64) -> Vec<u8>`.
- R11: The builder SHALL emit exactly 12 bytes: little-endian constructor `0x58e4a740` followed by exactly one little-endian signed `i64` request message id.
- R12: The builder SHALL preserve the full signed `i64` value without validation, clamp, reinterpretation, correlation, lookup, or history mutation.
- R13: Request serialization SHALL remain strictly separate from `parse_rpc_drop_answer` and the three response variants.
- R14: Stage-A contracts SHALL cover exact layout, signed `i64::MIN/i64::MAX` preservation, and request/response parser separation, but SHALL NOT be executed.
- R15: The serializer SHALL have no socket, scheduler, callback registry, request-history store, ACK/resend/container, persistence, timeout/reconnect, gzip, or updates-difference side effect.
- R16: Drop-answer scheduling, selection of a historical outstanding RPC, response correlation, and mutation of arbitrary `wasSent/haveSent/_resendingIds/toSend/_sentContainers` history remain fail-closed until an exact existing owner can represent them.

Additional acceptance criteria:

- AC7: The request constructor and public builder exist in the canonical protocol owner and desktop provider copies are synchronized byte-for-byte.
- AC8: Exact serializer contracts preserve constructor + one signed `i64` only.
- AC9: Request serialization stays distinct from `RpcDropAnswer` response parsing and does not claim request-history lifecycle semantics.
- AC10: Executable validation remains `not-run` and acceptance remains false in Stage A; historical drop-answer lifecycle, gzip, and complete difference decoding/application remain explicitly out of scope/fail-closed.
