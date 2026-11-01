# Telegram proxy settings state / persistence boundary — Specification

Status: active  
Owner: future canonical Fabushi Settings/application owner; no shipping owner currently exists for this state  
Last updated: 2026-11-01  
Related issue/task/PR: 7457eb1c-11a3-4626-8563-a3e495872536 round 82

## 1. Context / problem

Fixed `telegramdesktop/tdesktop@f985723b552b30fe1c17d377d57f44e61a15a8c4` carries a proxy-settings state responsibility that is adjacent to, but independent from, the WebProxy carrier and the already-reviewed WebProxy address/link boundary.

The fixed-U ownership chain is explicit:

- `Telegram/SourceFiles/core/core_settings_proxy.h@1f2ec5c516e03ae49e80c15bb6083bffa7ec6d99` declares `Core::SettingsProxy` and owns the persisted proxy mode, selected proxy, proxy list, rotation preferences/options, IPv6 preference, call-proxy flag and IP-warning bit.
- `Telegram/SourceFiles/core/core_settings_proxy.cpp@18e7ab8ce60865409dbaff267ee192e272622d26` owns mutation invariants plus its legacy serializer/deserializer.
- `Telegram/SourceFiles/core/core_settings.h@7192f0db82ddc214489f4c8bd5c247b8911b8a07` embeds exactly one `SettingsProxy _proxy` inside the application `Core::Settings` aggregate.
- `Telegram/SourceFiles/core/core_settings.cpp@6ef88cd5f0b946b0b92512d1a94e42a0f5c1fabb` embeds `_proxy.serialize()` as a byte array in application settings and restores it through `_proxy.setFromSerialized(proxy)`.
- `Telegram/SourceFiles/storage/localstorage.cpp@d71385083c0e940580398612fb65e522afc768b1` stores the whole `Core::Settings::serialize()` payload as encrypted `dbiApplicationSettings`.
- `Telegram/SourceFiles/boxes/connection_box.cpp@e9fa8a1d8f727042e4cadd632182eee5259f80c2` binds its controller to `Core::App().settings().proxy()`, mutates that one object and schedules `Local::writeSettings()`; its local list is a view/edit projection rather than a second state owner.
- `Telegram/SourceFiles/core/application.cpp@7172206f5c1e540e19c90798e9a9e5721276cc53` mutates selected+mode through the same `SettingsProxy`, refreshes the global proxy projection and emits change notifications.
- `Telegram/SourceFiles/core/proxy_rotation_manager.cpp@72723d4db408be654dbf68e1ac40bba21c59d4e6` consumes that state, owns only transient checking/switch timing, promotes preferred indices back into `SettingsProxy`, and persists through the application settings path. It is not the canonical settings owner.

This responsibility can be separated from sockets/carrier scheduling at the model/persistence level, but only if it is placed in the existing canonical Fabushi Settings/application owner. Creating a Telegram-specific proxy settings store would duplicate product settings truth.

## 2. Goal

Record the exact fixed-U proxy list / selected / mode / rotation / persistence responsibility, locate the unique source state owner, and determine whether the current Fabushi checkpoints contain an existing canonical settings/persistence owner that can absorb it without WebProxy carrier work or a parallel settings subsystem.

If the required owner is absent, keep the responsibility `mapped/pending` and author durable contracts/unblock conditions without adding dead or wrongly-owned production state.

## 3. Non-goals / out of scope

- WebProxy relay carrier, WebView bridge bootstrap, socket scheduling, reconnect/window lifecycle.
- The already-reviewed WebProxy Unicode/IDNA canonical-address and import/share boundary except where a persisted proxy value is referenced.
- Proxy reachability checking implementation or rotation scheduler implementation.
- Recreating Qt/QDataStream as Fabushi's long-term canonical settings storage format.
- A standalone `TelegramProxySettings`, `TelegramSettingsStore`, provider-local settings database, or second Settings UI/root.
- Stage-B build, dependency resolution, generator, lint, schema validation, tests, CI/status/check queries, package, deployment, installation or runtime acceptance.

## 4. Requirements

- R1: Preserve exact fixed-U provenance and distinguish canonical settings state from carrier/runtime scheduling.
- R2: The unique fixed-U persisted state owner is `Core::SettingsProxy`, embedded once in `Core::Settings`; UI/controller and rotation manager code are consumers/projections, not alternate owners.
- R3: Proxy mode has exactly three semantic states: `System`, `Enabled`, `Disabled`. The fixed-U legacy integer mapping is 0/1/2 respectively.
- R4: Persisted/settings fields are: `tryIPv6`, `useProxyForCalls`, `proxyRotationEnabled`, `checkIpWarningShown`, `proxyRotationTimeout`, `settings`, `selected`, `list`, and `proxyRotationPreferredIndices`. Fixed-U default rotation timeout is 10 seconds; `tryIPv6` defaults false on Windows and true elsewhere.
- R5: `setProxyRotationTimeout(value)` maps every non-positive value to 10 and preserves positive values.
- R6: Preferred rotation indices are canonicalized by dropping negative, out-of-range and duplicate entries while preserving first-occurrence order.
- R7: List mutation invariants are exact: replacing the whole list clears preferred indices; inserting at index clamps to the list bounds and increments every preferred index at/after the insertion; removing decrements every preferred index after the removed position and removes the removed position itself.
- R8: Promoting a valid preferred index is idempotent when already first, moves an existing later index to the front, or inserts an absent valid index at the front; invalid indices do nothing.
- R9: Fixed-U legacy proxy record serialization maps proxy type `None/Socks5/Http/Mtproto/Web` to 0/1/2/3/4 and stores type, host, port, user and password. Unknown legacy proxy type decodes as `None`.
- R10: Legacy settings deserialization is append-compatible: empty payload succeeds; later fields are read only when present; negative list/preferred counts fail; unknown/`None` list entries are skipped; preferred indices are remapped around skipped entries; an unknown/`None` selected proxy becomes empty; malformed stream status fails.
- R11: Long-term Fabushi canonical persistence may use the unified source-neutral settings schema, but any TDesktop-settings migration/import adapter must preserve R3-R10 semantics. The Qt byte layout must not become a new parallel canonical store.
- R12: Selection/mode changes must be one atomic domain mutation from the perspective of Settings consumers; downstream networking receives a projection/event from the canonical owner. A carrier or proxy checker must never become the state owner.
- R13: Existing-owner-first is mandatory. Current `mahayana-app-host` `feature.persistence.*` is a UI/client key-value persistence surface backed by a table literally named `ui_state`; current Telegram encrypted SQLite is Telegram runtime/auth/transport private persistence; current Fabushi account-service persists account/session and protected Telegram storage-key material. None is a canonical product Settings owner for proxy list/selection/rotation.
- R14: Do not add a second settings subsystem merely because the pure model is carrier-independent. If no existing canonical Settings owner is shipping in the checkpoint, record blocked/no-production-delta.
- R15: Stage A may author durable spec/contract vectors/bookkeeping only for a blocked owner. Executable validation remains `not-run`.

## 5. Current state

Current live checkpoints at discovery start:

- `bhrumom/fabushi-desktop checkpoint/tdrp-stage-a-msg-mutations-7457eb1c-r1@757a1dc07e9c79a98f96703bb1cbd921c197b81d`
- `bhrumom/fabushi-platform-core checkpoint/tdrp-telegram-auth-lifecycle-7457eb1c-r16@2682d35b46d5521d71ea82896557466c3e07a58f`

Fabushi candidate review:

1. Desktop `source/mahayana/mahayana-rs/mahayana-app-host/src/lib.rs@ed69b2b7aa0d87ebd34210e9a13d9c8a8b3e3087` exposes `feature.persistence.get/set/remove/list`, but `runtime_state_connection()` creates `feature-host/runtime/runtime.sqlite3` and a `ui_state(key,value_json,updated_at_ms)` table. This is generic UI/client feature persistence, not a canonical Settings state/lifecycle owner.
2. Desktop/platform Telegram storage `sqlite.rs@e6cc314ec749f8a258bfe0125a25beb3613eb2ca` is an encrypted Telegram runtime store and is already used for authorization/transport private state. Making it own product proxy settings would create provider-local settings truth.
3. Desktop `source/product/fabushi/fabushi-account-service.ts@8e26fd5eab70d27ee553791ca30a8d6147358bf5` owns account/session persistence and protected Telegram storage-key material, not global application settings.
4. The platform-core first-party tree at this checkpoint has no shipping general Settings state owner path. Its non-third-party settings/config-like production paths are service/runtime-specific (for example Global Dharma config and Telegram storage), not the canonical desktop product Settings root.
5. `third_party/mahayana/codex-rs/config` is Agent/Codex configuration code and is not the canonical Fabushi desktop Settings composition/state owner for communication/network preferences.

Therefore the exact state responsibility is conceptually carrier-independent but is not independently shippable at this checkpoint without inventing a new settings owner or abusing an existing persistence surface.

## 6. Target state

The eventual canonical shape is:

1. one Fabushi Settings/application owner stores typed `ProxySettingsState`;
2. it owns list/selection/mode/rotation invariants from R3-R10;
3. it persists through the existing unified product settings persistence/recovery path;
4. the existing Settings UI projects/edits that state through typed commands/events;
5. networking/carrier code consumes a canonical selected-proxy projection and emits connection observations only;
6. a rotation capability may consume/update preferred order through the Settings owner, but its timers/checkers remain transient runtime state;
7. optional TDesktop migration code reads legacy Qt payloads into the canonical schema without making the legacy format the new storage truth.

## 7. Architecture and ownership boundaries

Required owner split:

- canonical Fabushi Settings/application owner: authoritative proxy settings state, mutation invariants, persistence/restart and user-facing command/event boundary;
- canonical Telegram protocol/config primitives: typed proxy value semantics already reviewed in adjacent specs;
- Telegram/network carrier: connection transport only;
- rotation/checker runtime: transient reachability/timer/switch execution only;
- Settings UI: projection/edit surface only.

Rejected owners:

- `mahayana-app-host::feature.persistence`: generic UI-state persistence, not domain Settings truth.
- `telegram-storage`: Telegram runtime private store; using it would create a provider-specific settings subsystem.
- `fabushi-account-service`: account/session owner; proxy preferences are not identity/session credentials.
- `telegram-network`: transport lifecycle, not product settings.
- `proxy_rotation_manager` equivalent: scheduler/consumer, not persisted settings owner.
- a new Telegram-specific settings crate/store/UI root: violates unified settings architecture.

## 8. Interfaces / contracts / schemas / data flow

Required future source-neutral model:

`SettingsCommand -> canonical ProxySettingsState reducer -> durable unified settings persistence -> SettingsEvent/selected-proxy projection -> network/rotation consumers`

Minimum typed state:

- mode: `System | Enabled | Disabled`
- selected: optional canonical proxy value
- list: ordered canonical proxy values
- try_ipv6: bool
- use_proxy_for_calls: bool
- check_ip_warning_shown: bool
- rotation_enabled: bool
- rotation_timeout_seconds: positive integer with default 10
- rotation_preferred_indices: normalized ordered integer list

Minimum mutation commands include list replace/add/insert/remove/replace, selected/mode update, rotation toggle/timeout/preference promotion, IPv6/call-proxy/IP-warning updates.

No carrier handle, socket, WebView, checker or timer is stored in the settings model.

## 9. Constraints and non-functional requirements

- Single authoritative settings state and persistence path.
- Deterministic mutation semantics.
- Fail closed on malformed migration data.
- No proxy secret logging.
- No provider-local or UI-local duplicate truth.
- Persistence/restart semantics must be explicit before implementation.
- Stage-A zero-execution rules remain in force.

## 10. Failure modes and edge cases

- selected proxy is removed from the list;
- duplicate list values arrive from UI/import;
- legacy list contains unknown proxy type;
- legacy preferred index points at an unknown/skipped proxy;
- negative list/preferred count;
- malformed/truncated legacy stream;
- zero/negative rotation timeout;
- preferred index duplicate/out of range;
- provider runtime starts before Settings restoration;
- UI-state persistence is accidentally treated as canonical settings;
- carrier or rotation runtime writes state outside the Settings owner.

## 11. Implementation strategy

This round is a deliberate blocked/no-production-delta disposition:

1. retain all current production blobs unchanged;
2. record one new exact responsibility as `mapped/pending`;
3. author the state/mutation/migration contract vectors below;
4. update the unique desktop ledger/P0/source-closure/evidence;
5. unblock only when the canonical Fabushi Settings/application owner and its persistence/restart contract are present in the shipping checkpoint.

No minimal new owner ADR is created because the master architecture explicitly requires one canonical Settings root and forbids source-specific parallel settings systems.

## 12. Verification / test strategy

Authored contracts only; none are executed in Stage A.

Contract set `CONTRACT-TDRP-PROXY-SETTINGS-STATE-001`:

- default state: rotation timeout 10; platform-specific IPv6 default; System mode; empty selected/list/preferred;
- mode legacy mapping 0/1/2 and invalid mode fails closed;
- timeout <= 0 normalizes to 10;
- preferred normalization removes [-1, out-of-range, duplicate] while preserving valid first order;
- list insert at beginning/middle/end adjusts preferred indices exactly;
- list remove adjusts preferred indices exactly and removes the removed index;
- list replacement clears preferred order;
- promote preferred index front/idempotent/insert behavior;
- legacy proxy type map 0..4 and unknown -> None;
- unknown legacy list entry is skipped and preferred indices are remapped;
- unknown selected becomes empty;
- empty legacy payload succeeds;
- append-compatible missing later fields preserve defaults;
- negative counts and malformed/truncated payload fail;
- `connection_box` projection never becomes a second list owner;
- rotation runtime may promote/persist preference through Settings but never owns durable settings;
- production proxy/settings/network blobs remain unchanged in this blocked round.

No executable contract is added against a nonexistent canonical Settings API merely to manufacture coverage.

## 13. Acceptance criteria / Definition of Done

- AC-1: fixed-U source identities and unique SettingsProxy state ownership are durably recorded.
- AC-2: exact list/selected/mode/rotation/persistence invariants are recorded separately from carrier scheduling.
- AC-3: current Fabushi persistence candidates are inspected and rejected/accepted with exact owner reasons.
- AC-4: no parallel Telegram settings store, UI-state abuse or carrier-owned settings state is introduced.
- AC-5: unique desktop ledger row is `mapped/pending`, `validation_status=not-run`, `accepted=false`, with explicit unblock conditions.
- AC-6: desktop P0/source-closure/evidence record exact source and owner evidence plus unchanged production blobs.
- AC-7: final platform spec compliance references desktop bookkeeping and records all source-review outcomes.
- AC-8: Stage-A executable validation remains not-run.

## 14. Release / migration / rollback

No runtime release, persistence migration, dependency or production change occurs in this round. Rollback is removal of this spec/bookkeeping evidence only.

## 15. Observability / evidence

Evidence is Git identity and source review:

- fixed-U source blobs listed in section 16;
- live platform/desktop checkpoint refs;
- desktop AppHost/UI-state persistence blob;
- desktop/platform Telegram storage blob;
- desktop account-service blob;
- initial/final platform spec commits and blobs;
- desktop unique ledger/P0/source-closure/evidence commits/blobs;
- unchanged production proxy/protocol/network/storage blobs.

## 16. References / provenance

Fixed U:

- `telegramdesktop/tdesktop@f985723b552b30fe1c17d377d57f44e61a15a8c4`
- `Telegram/SourceFiles/core/core_settings_proxy.h@1f2ec5c516e03ae49e80c15bb6083bffa7ec6d99`
- `Telegram/SourceFiles/core/core_settings_proxy.cpp@18e7ab8ce60865409dbaff267ee192e272622d26`
- `Telegram/SourceFiles/core/core_settings.h@7192f0db82ddc214489f4c8bd5c247b8911b8a07`
- `Telegram/SourceFiles/core/core_settings.cpp@6ef88cd5f0b946b0b92512d1a94e42a0f5c1fabb`
- `Telegram/SourceFiles/boxes/connection_box.h@02717ab7b646089aa055c441b86e18e35d156e09`
- `Telegram/SourceFiles/boxes/connection_box.cpp@e9fa8a1d8f727042e4cadd632182eee5259f80c2`
- `Telegram/SourceFiles/core/application.cpp@7172206f5c1e540e19c90798e9a9e5721276cc53`
- `Telegram/SourceFiles/core/proxy_rotation_manager.h@c1fee276961c4f98e21ab2a8e495a1d687672804`
- `Telegram/SourceFiles/core/proxy_rotation_manager.cpp@72723d4db408be654dbf68e1ac40bba21c59d4e6`
- `Telegram/SourceFiles/storage/localstorage.h@ca847ab41eb2614b2d56ea248e932ab8ebead9ec`
- `Telegram/SourceFiles/storage/localstorage.cpp@d71385083c0e940580398612fb65e522afc768b1`
- `Telegram/SourceFiles/mtproto/mtproto_proxy_data.h@d8deebac2359936f72901ba0eec0cbfdfb186e90`

Current Fabushi evidence at discovery start:

- desktop AppHost `mahayana-app-host/src/lib.rs@ed69b2b7aa0d87ebd34210e9a13d9c8a8b3e3087`
- desktop account service `source/product/fabushi/fabushi-account-service.ts@8e26fd5eab70d27ee553791ca30a8d6147358bf5`
- desktop/platform Telegram storage `sqlite.rs@e6cc314ec749f8a258bfe0125a25beb3613eb2ca`
- adjacent WebProxy address/settings spec `docs/specs/telegram-web-proxy-canonical-address-settings-boundary.md@eb924c6492ffb4f545b9ccc2eec404284c41335d`

No upstream C++ is copied into Fabushi.

## 17. Spec compliance record

Initial source disposition. Executable validation intentionally remains not-run.

| Requirement / AC | Status | Evidence / reason |
| --- | --- | --- |
| R1-R12 | passed | Fixed-U source review establishes unique owner, exact state/mutation/legacy persistence semantics and separation from carrier runtime. |
| R13-R14 | blocked | Existing persistence surfaces are not the canonical product Settings owner; no shipping unified Settings state owner is present in the current checkpoint. |
| R15 | passed | Only durable contracts/bookkeeping are allowed; no executable validation is run. |
| AC-1-AC-4 | passed | Source/owner review is complete and no production state is added. |
| AC-5-AC-7 | pending | Await desktop bookkeeping and final platform readback. |
| AC-8 | passed | Stage-A executable validation remains not-run. |
