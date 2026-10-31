# Canonical message retention / auto-delete — Specification

Status: active  
Owner: Fabushi Platform Core / `native/mahayana-messaging`  
Last updated: 2026-10-13  
Related issue/task/PR: TDRP-001 Revision 9, deterministic source orders 520-522 and related message-TTL responsibilities

## 1. Context / problem

Telegram Desktop's accepted source authority `telegramdesktop/tdesktop@63a1831728147e941ad2dc8939879cf5afe419c7` carries conversation-level message TTL / auto-delete semantics through `PeerData::messagesTTL`, TTL validation/settings UI, server update application, and composer/history projections. The canonical Fabushi shared messaging model currently has no source-neutral conversation retention policy and no message expiry timestamp. Desktop must not create a Telegram-specific TTL owner or local-only timer.

## 2. Goal

Add a source-neutral, authoritative message-retention contract to canonical Mahayana messaging so every platform can expose conversation auto-delete without creating a second message/conversation store.

## 3. Non-goals / out of scope

- No Telegram protocol, MTProto, Telegram artwork, or Telegram-specific runtime.
- No platform-specific picker/UI in Platform Core.
- No deletion based only on client wall-clock timers.
- No retroactive deletion semantics beyond the explicitly selected policy.
- No weakening of existing durable sync, idempotency, authorization, or Bot behavior.

## 4. Requirements

- **RET-R1 Authoritative policy:** a conversation may carry an optional message retention period in seconds; `None` disables automatic retention. The server-authoritative conversation state is the source of truth.
- **RET-R2 Bounded values:** non-null retention must be validated against a shared bounded policy. Invalid values fail closed and are not persisted.
- **RET-R3 Authorization:** only actors already authorized to manage conversation policy may change retention; direct/group/channel ownership and admin rules must reuse canonical authorization.
- **RET-R4 Message expiry identity:** messages created while a retention policy is active receive an immutable `expires_at_ms` derived from authoritative server time and the policy effective for that message. Messages created while disabled have no expiry.
- **RET-R5 No rewrite on policy change:** changing the conversation policy affects subsequently created messages only unless a future explicit migration spec authorizes retroactive behavior.
- **RET-R6 Deterministic expiry:** expiry is settled by the authoritative messaging service before sync/search/read projections. Expired content must not reappear after restart, pagination, reconnect, or multi-device sync.
- **RET-R7 Durable tombstone semantics:** expiry transitions reuse the canonical message deletion/tombstone path so cursors, event journals, dedupe/idempotency, references and restart recovery remain coherent.
- **RET-R8 Stale fencing:** a stale client cannot overwrite a newer retention policy without an authority/version check. The contract must expose enough revision/state identity for platforms to reject or reconcile stale edits.
- **RET-R9 Multi-device convergence:** all devices observing the same conversation converge on the same policy and expiry transitions through the existing sync/event journal.
- **RET-R10 Bot/no-regression:** Agent/Bot conversations and typed Bot entries keep existing behavior; retention must not create a separate Bot message transport or bypass canonical Bot dispatch.
- **RET-R11 Source-neutral projection:** platform UIs consume the canonical policy and mutation contract; desktop may map it to ConversationComposer/Menu/Settings using Fabushi components.
- **RET-R12 Observability:** policy changes and expiry settlements provide traceable event/cursor evidence suitable for exact-head CI and service E2E.

## 5. Current state

`Conversation` has no retention policy. `Message` has no expiry timestamp. `MessagingService` persists snapshots and an event journal and already centralizes authorization, sync, search, idempotent send replay and canonical deletion, but only typing/credential-style TTLs exist today.

## 6. Target state

One canonical retention policy is persisted with Conversation state; one canonical expiry field is persisted with Message state; one authority mutation changes policy; one deterministic settlement path tombstones expired messages and journals the resulting change. Desktop/iOS/Android/web project the same contract.

## 7. Architecture and ownership boundaries

- **Owner:** `native/mahayana-messaging` in `bhrumom/fabushi-platform-core`.
- **Conversation state:** retention policy and policy revision.
- **Message state:** immutable expiry timestamp.
- **Engine:** validated commands/events and tombstone transition.
- **Service:** authorization, authoritative server-time derivation, stale-revision fencing, expiry settlement before externally visible projections, durable persistence/journal.
- **Platform consumers:** UI only; no local truth or second timer/store.
- **Backend deployment:** whichever canonical service hosts Mahayana messaging must run this exact contract; mocks do not satisfy production acceptance.

## 8. Interfaces / contracts / schemas / data flow

Target contract shape (names may be refined during implementation without changing semantics):

`Conversation.message_retention_seconds: Option<u32>`  
`Conversation.retention_revision: u64`  
`Message.expires_at_ms: Option<i64>`

Authority mutation:

`SetConversationMessageRetention { conversation_id, retention_seconds, expected_revision }`

Authority event:

`ConversationMessageRetentionUpdated { conversation_id, retention_seconds, revision, changed_at_ms }`

Flow:

policy mutation -> authorization + expected-revision check -> engine event -> snapshot/journal -> platform sync;  
send -> read current policy -> derive immutable expiry from server time -> queue message -> snapshot/journal;  
handle/sync/search -> settle due expiries -> canonical tombstones/events -> persist journal -> project only non-expired content where appropriate.

## 9. Constraints and non-functional requirements

- Overflow-safe time arithmetic.
- Stable serialization/backward compatibility for snapshots produced before the fields exist.
- No unbounded expiry scanning on hot paths; implementation must use a bounded/deterministic strategy appropriate to current state size and evolve to an index if needed.
- Exact same ordering/cursor guarantees as ordinary message deletion.
- Builds/tests only in GitHub Actions for this migration work.

## 10. Failure modes and edge cases

- invalid/zero/overflowing retention value;
- unauthorized policy mutation;
- stale expected revision;
- message created exactly at a policy-change boundary;
- restart before/after expiry settlement;
- duplicate/replayed send request;
- pinned/replied/forwarded expired message;
- scheduled message whose send time is later than compose time;
- offline device reconnecting after multiple expiries;
- clock regression: authority uses supplied monotonic service observation semantics and saturating arithmetic, never client clock as truth.

## 11. Implementation strategy

1. Extend serializable Conversation/Message schema with backward-compatible defaults.
2. Add engine command/event for policy mutation and expiry tombstone settlement.
3. Extend protocol with source-neutral mutation/event.
4. Reuse service authorization; require expected revision and validate bounds.
5. Derive expiry at authoritative message creation/acceptance time.
6. Settle due expiries before sync/search and other externally visible message projections; persist/journal atomically.
7. Add focused contract/unit/state/restart/idempotency/stale-revision tests.
8. Wire canonical desktop projection in `bhrumom/fabushi-desktop` only after the core contract is merged/pinned.

## 12. Verification / test strategy

All executable verification runs in GitHub Actions.

- serialization/backward-compatibility tests;
- valid/invalid retention policy tests;
- owner/admin authorization tests;
- stale expected-revision rejection;
- send-before/send-after policy transition;
- exact expiry-boundary test;
- duplicate send replay preserves one expiry identity;
- restart loads policy/message expiry and settles correctly;
- sync/search do not expose already-expired live content;
- multi-actor journal/cursor convergence;
- Bot invocation regression for retained messages and no invocation from already-expired content;
- integration test with an actual canonical messaging service process, not a fake response.

## 13. Acceptance criteria / Definition of Done

- **RET-AC1:** canonical Conversation + Message schemas represent retention/expiry with backward-compatible serialization.
- **RET-AC2:** policy mutation is authorized, revision-fenced and durably journaled.
- **RET-AC3:** messages receive deterministic server-authoritative expiry identity.
- **RET-AC4:** expiry uses the canonical tombstone path and survives restart/reconnect/multi-device sync without resurrection.
- **RET-AC5:** search/sync projections exclude already-expired live content.
- **RET-AC6:** existing Bot/Agent message, idempotency, pagination and recovery tests remain green.
- **RET-AC7:** Platform Core exact-head GitHub Actions are green and artifact/evidence identity is recorded.
- **RET-AC8:** a canonical service E2E demonstrates policy change -> send -> expiry -> restart/reconnect -> no resurrection.
- **RET-AC9:** desktop consumes the merged core contract through existing Conversation/Composer owners; no Telegram runtime/second store is introduced.
- **RET-AC10:** TDRP source orders 520-522 remain open until RET-AC1..9 have production/current-head evidence; source read evidence alone is insufficient.

## 14. Release / migration / rollback

Schema additions must deserialize old snapshots using defaults. Rollback to a binary that cannot understand new authoritative retention semantics is not a supported production rollback after the feature is enabled; rollout therefore remains disabled until all shipping consumers required by the release candidate understand the contract. Before enablement, rollback is code-only and policy remains unset.

## 15. Observability / evidence

Record exact Platform Core SHA, CI run/jobs, test counts, service E2E evidence, desktop consumer SHA, and TDRP source-disposition bindings. Expiry and policy events must carry conversation/message identity and cursor without logging message content or credentials.

## 16. References / provenance

- `telegramdesktop/tdesktop@63a1831728147e941ad2dc8939879cf5afe419c7`
- `Telegram/Resources/icons/chat/input_autodelete{,@2x,@3x}.png` source orders 520-522
- `Telegram/SourceFiles/chat_helpers/chat_helpers.style::historyMessagesTTL`
- `Telegram/SourceFiles/data/data_peer.{h,cpp}::messagesTTL/setMessagesTTL`
- `Telegram/SourceFiles/history/view/controls/history_view_ttl_button.cpp`
- `Telegram/SourceFiles/menu/menu_ttl_validator.cpp`
- `Telegram/SourceFiles/ui/boxes/auto_delete_settings.cpp`
- Fabushi canonical `native/mahayana-messaging` Conversation/Message/Engine/Protocol/Service owners

## 17. Spec compliance record

| Requirement / AC | Status | Evidence / reason |
| --- | --- | --- |
| RET-R1..RET-R12 | blocked | Durable spec established; production implementation not yet merged. |
| RET-AC1..RET-AC10 | blocked | Requires implementation plus exact-head Actions and real service evidence. |
