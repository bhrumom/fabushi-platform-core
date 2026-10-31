# Telegram MTProto new-session exact wire validation — Specification

Status: active  
Owner: shared Telegram protocol/network runtime  
Last updated: 2026-11-01  
Related task: Fabushi migration task 7457eb1c-11a3-4626-8563-a3e495872536 round 25

## 1. Context / problem

The existing `telegram-network::EstablishedSession` already handles fixed-U `new_session_created#9ec20908` for the scoped single-RPC replay path, but it reads fields by raw byte offsets. That path currently consumes `first_msg_id` and `server_salt` without a shared typed parser that also validates `unique_id` and exact buffer consumption.

## 2. Goal

Move complete fixed-U `new_session_created` wire validation into the existing `telegram-protocol/src/updates.rs` owner and make both current RPC and ping/service receive paths use it before any salt/replay side effect.

## 3. Non-goals / out of scope

- Higher-level update/difference recovery after the service message.
- Historical multi-request replay ownership.
- `gzip_packed` decompression.
- A second session/runtime/provider.

## 4. Requirements

- R1: `telegram-protocol/src/updates.rs` SHALL expose the fixed-U constructor `0x9ec20908`.
- R2: It SHALL expose a typed value containing `first_message_id`, `unique_id`, and `server_salt`.
- R3: The parser SHALL reject wrong constructors, truncation, and trailing bytes.
- R4: `EstablishedSession::scan_rpc_body` SHALL parse the complete typed object before updating salt or deciding current-RPC replay.
- R5: `EstablishedSession::process_service_body` SHALL parse the same typed object before updating salt on the ping/service path.
- R6: Existing bounded replay semantics remain unchanged: only the current outstanding RPC may be retried, and only once.
- R7: No historical `wasSent/haveSent/_resendingIds/toSend/_sentContainers` owner SHALL be inferred or introduced.
- R8: Stage-A exact-field/malformed-payload contracts SHALL be authored but not executed.
- R9: Desktop provider copies SHALL be synchronized byte-for-byte with canonical platform-core protocol/network blobs.
- R10: This slice SHALL NOT claim the separate high-level update/difference recovery responsibility complete.

## 5. Current state

- `telegram-protocol/src/updates.rs` owns update-state/get-difference wire primitives but has no new-session parser.
- `telegram-network/src/lib.rs` has a private duplicate `NEW_SESSION_CREATED_CONSTRUCTOR` and offset reads at byte 4/20.
- Both `invoke_raw` and ping/service receive paths are already in the shipping account-bound transport chain.

## 6. Target state

Authenticated/decrypted service body -> shared typed `parse_new_session_created` -> exact constructor/three-field/trailing validation -> existing salt update -> existing bounded replay or continue.

## 7. Architecture and ownership boundaries

Wire structure belongs to `telegram-protocol`; transport side effects remain in `telegram-network::EstablishedSession`. Runtime/account ownership does not change.

## 8. Interfaces / contracts / schemas / data flow

Fixed-U authority: `new_session_created#9ec20908 first_msg_id:long unique_id:long server_salt:long = NewSession`.

## 9. Constraints and non-functional requirements

- Fail closed before side effects.
- No new dependency.
- No CI/build/test execution in Stage A.

## 10. Failure modes and edge cases

Wrong constructor, fewer than three longs, or any trailing bytes fail through the existing protocol/network invalid-service-message path. A valid object preserves the existing one-RPC replay comparison and ping salt adoption.

## 11. Implementation strategy

Add the typed parser and inline authored unit contracts to `updates.rs`; re-export it from protocol `lib.rs`; replace the network-local constructor/offset reads in both new-session branches with the typed parser.

## 12. Verification / test strategy

Author but do not run:
- exact three-field parse;
- wrong-constructor rejection;
- truncation rejection;
- trailing-byte rejection;
- existing replay predicate contracts remain unchanged.

## 13. Acceptance criteria / Definition of Done

- AC-1: canonical protocol parser and network consumer are committed on the existing checkpoint;
- AC-2: parsing precedes salt/replay side effects in both receive paths;
- AC-3: desktop provider blobs match canonical blobs;
- AC-4: ledger/P0/source-closure record implemented/not-run/accepted=false;
- AC-5: high-level difference recovery remains explicitly blocked/open;
- AC-6: no Stage-A executable validation is performed.

## 14. Release / migration / rollback

No release in Stage A. Revert the implementation commit if later Stage-B validation finds a defect.

## 15. Observability / evidence

Git blob identities, commit identities, manual source review, and authored contracts only.

## 16. References / provenance

- `telegramdesktop/tdesktop@f985723b552b30fe1c17d377d57f44e61a15a8c4:Telegram/SourceFiles/mtproto/session_private.cpp@13cbc76ec723386027b5b4dcac79999b791c4614`
- `telegramdesktop/tdesktop@f985723b552b30fe1c17d377d57f44e61a15a8c4:Telegram/SourceFiles/mtproto/scheme/mtproto.tl@76225965bfc794388db241f3f760db8ab79e84f8`

## 17. Spec compliance record

| Requirement / AC | Status | Evidence / reason |
| --- | --- | --- |
| R1-R10 | blocked | Spec authored before implementation. |
| AC-1..AC-6 | blocked | Implementation/desktop sync/bookkeeping not yet applied at this checkpoint. |
