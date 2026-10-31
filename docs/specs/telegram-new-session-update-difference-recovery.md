# Telegram new-session update/difference recovery — Specification

Status: active  
Owner: shared Telegram protocol/runtime update synchronization  
Last updated: 2026-11-01  
Related task: Fabushi migration task 7457eb1c-11a3-4626-8563-a3e495872536 round 25

## 1. Context / problem

Fixed upstream `telegramdesktop/tdesktop@f985723b552b30fe1c17d377d57f44e61a15a8c4` separates low-level MTProto new-session handling from high-level update recovery. `SessionPrivate::handleOneReceived / mtpc_new_session_created` adopts the new salt, performs request replay owned by the transport, then forwards the service message to the main update owner. `Api::Updates::mtpNewSessionCreated` resets update sequence state and immediately calls `updates.getDifference`; non-empty difference responses are applied before the update state advances, and slice responses continue until a terminal state is reached.

Fabushi already owns the low-level single-outstanding-RPC replay subset, but the service message is not propagated to a complete difference-application owner.

## 2. Goal

Preserve the fixed-U high-level recovery contract in the existing shared Telegram protocol/runtime stack: a validated `new_session_created` must cause the canonical update owner to recover missed updates through `updates.getDifference` without creating a second runtime, request queue, message store, or Telegram product root.

## 3. Non-goals / out of scope

- Historical `wasSent/haveSent/_resendingIds/toSend/_sentContainers` request bookkeeping.
- `gzip_packed` decompression.
- Channel-specific difference recovery.
- Inventing a second Telegram update/message store or renderer-owned protocol state.

## 4. Requirements

- R1: The existing encrypted-session receive path SHALL surface a validated new-session recovery signal to the existing runtime/update owner after low-level salt/replay handling.
- R2: The high-level update owner SHALL reset sequence continuity to the fixed-U new-session baseline before requesting a difference.
- R3: Recovery SHALL issue `updates.getDifference` from the current canonical `pts/date/qts` state.
- R4: `updates.differenceEmpty` SHALL preserve current `pts/qts` while advancing the returned `date/seq`.
- R5: `updates.differenceSlice` SHALL apply its messages/updates/chats/users into existing canonical Telegram domain owners, advance to `intermediate_state`, and request the next difference.
- R6: `updates.difference` SHALL apply its messages/updates/chats/users before committing the returned final state.
- R7: Unsupported/malformed difference payloads SHALL fail closed; they SHALL NOT be discarded while update state advances.
- R8: Recovery SHALL reuse the existing `telegram-protocol`, `telegram-runtime`, `telegram-core`, and account-bound transport composition.
- R9: Stage-A verification contracts may be authored but SHALL NOT be executed.

## 5. Current state

Verified on the round-25 starting heads:

- `telegram-network::EstablishedSession` recognizes `new_session_created`, adopts `server_salt`, and performs the already-scoped single-RPC replay decision, but does not expose a high-level update-recovery event.
- `telegram-protocol/src/updates.rs` can build `updates.getState` / `updates.getDifference`, parse `updates.state`, and parse only terminal `differenceEmpty` / `differenceTooLong`.
- `telegram-runtime::RuntimeClient` owns `update_state: Option<UpdateState>`; `telegram.beginUpdateSync` currently obtains only `updates.getState`.
- `telegram-core::TelegramEngine` has canonical chat/message commands such as `UpsertRemoteMessage`, but there is no current TL decoder/adapter that can consume arbitrary `Message`, `EncryptedMessage`, `Update`, `Chat`, and `User` objects embedded in `updates.difference` / `updates.differenceSlice`.

## 6. Target state

Authenticated new-session event -> existing EstablishedSession low-level salt/replay -> typed recovery signal -> existing RuntimeClient update owner resets sequence continuity -> `updates.getDifference` -> decode/apply canonical domain changes -> commit intermediate/final update state -> continue slices until terminal.

## 7. Architecture and ownership boundaries

`telegram-protocol` owns exact TL wire decoding/building. `telegram-network` owns authenticated transport/service-message handling. `telegram-runtime` owns account-bound update synchronization orchestration. `telegram-core` owns canonical chat/message state transitions. No renderer or second provider owns Telegram update truth.

## 8. Interfaces / contracts / schemas / data flow

Fixed-U schema authority:

- `updates.state#a56c2a3e pts:int qts:int date:int seq:int unread_count:int`
- `updates.differenceEmpty#5d75a138 date:int seq:int`
- `updates.difference#f49ca0 ... state:updates.State`
- `updates.differenceSlice#a8fb1981 ... intermediate_state:updates.State`
- `updates.differenceTooLong#4afe8f6d pts:int`
- `updates.getDifference#19c2f763 ... = updates.Difference`

The full/slice vectors are not self-delimiting without constructor-specific decoding, so a terminal-only parser cannot safely skip them to extract state.

## 9. Constraints and non-functional requirements

- Stage A remains zero CI and zero local execution.
- No lossy advancement of update state.
- No unbounded background worker is introduced.
- Existing account/transport ownership remains unique.

## 10. Failure modes and edge cases

The current implementation cannot satisfy R5/R6 because the canonical protocol/runtime stack lacks a complete decoder/application bridge for arbitrary objects inside full/slice differences. Sending `updates.getDifference` and accepting only `differenceEmpty` would be incomplete and could strand or drop real missed updates. Advancing state past an un-applied non-empty difference is forbidden.

This responsibility therefore remains fail-closed/blocked until the missing decoder/application bridge exists in existing owners.

## 11. Implementation strategy

Do not implement a partial automatic recovery path. First complete the source-neutral Telegram TL difference object decoder and canonical command adapter, then wire the already-existing new-session transport signal into the runtime recovery loop.

Independent service-message wire-validation responsibilities may proceed while this blocker remains.

## 12. Verification / test strategy

When unblocked, author contracts for direct and container-nested new-session propagation, empty/full/slice differences, malformed payloads, slice continuation, state advancement ordering, restart/recovery, and no duplicate canonical messages. Stage A does not execute them.

## 13. Acceptance criteria / Definition of Done

- AC-1: New-session service handling reaches the existing runtime update owner.
- AC-2: Full/slice difference payloads are decoded and applied before state advancement.
- AC-3: Empty difference state semantics match fixed U.
- AC-4: No second runtime/update/message owner is introduced.
- AC-5: Desktop provider composition consumes the same canonical implementation.
- AC-6: Stage-A ledger/evidence records implementation separately from validation/acceptance.

## 14. Release / migration / rollback

No release in Stage A. Rollback is ordinary commit reversion before the Stage-B candidate freeze.

## 15. Observability / evidence

Source/ref/blob reads and authored contracts are evidence only. Build/test/runtime evidence remains deferred to Stage B.

## 16. References / provenance

- `telegramdesktop/tdesktop@f985723b552b30fe1c17d377d57f44e61a15a8c4:Telegram/SourceFiles/mtproto/session_private.cpp@13cbc76ec723386027b5b4dcac79999b791c4614`
- `telegramdesktop/tdesktop@f985723b552b30fe1c17d377d57f44e61a15a8c4:Telegram/SourceFiles/api/api_updates.cpp@71c929cfe3c779c94719efef32a47a4d029e2c67`
- `telegramdesktop/tdesktop@f985723b552b30fe1c17d377d57f44e61a15a8c4:Telegram/SourceFiles/mtproto/scheme/api.tl@8319b7df634d145390c299404129fd161a5dbd2a`

## 17. Spec compliance record

| Requirement / AC | Status | Evidence / reason |
| --- | --- | --- |
| R1-R4 | blocked | New-session is consumed inside the network owner and is not propagated to a complete runtime difference loop. |
| R5-R7 | blocked | No existing full/slice TL object decoder/application bridge exists; terminal-only parsing is insufficient and lossy advancement is forbidden. |
| R8 | passed | The required existing owners are present; no new owner is proposed. |
| R9 | passed | Round 25 discovery/spec work performs source/Git reads and writes only; no executable validation is run. |
| AC-1..AC-6 | blocked | High-level recovery remains intentionally fail-closed pending the missing decoder/application bridge. |
