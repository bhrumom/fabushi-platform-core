# Telegram proxy non-UI consumers boundary

Status: Stage A source review / blocked-no-production-delta
Date: 2026-11-01
Fixed upstream: `telegramdesktop/tdesktop@f985723b552b30fe1c17d377d57f44e61a15a8c4`

## 1. Scope

This contract isolates three non-UI consumers of the fixed-U `Core::SettingsProxy` state without creating a second settings owner or smuggling carrier scheduling into a pure model:

1. external local-automation proxy mutation in `core/external_control.cpp`;
2. `useProxyForCalls` consumption in `calls/calls_call.cpp`;
3. MTProto selected-proxy / proxy-enabled consumption in `mtproto/mtp_instance.cpp`, including its direct `ConfigLoader` consequences.

The canonical durable proxy settings owner remains the single `Core::SettingsProxy` embedded in `Core::Settings`; this spec does not reopen or replace the prior proxy-settings state/persistence disposition.

## 2. Fixed-U source identities

- `Telegram/SourceFiles/core/external_control.cpp@43327bc03c528b65b41ce72add5e7cca584ea371`
- `Telegram/SourceFiles/calls/calls_call.cpp@a67723f5827e56be3f36033f3ad59bc37d0acf7b`
- `Telegram/SourceFiles/mtproto/mtp_instance.cpp@d25fe3b72298fab371854f80186ee8e4777ea087`
- `Telegram/SourceFiles/mtproto/config_loader.h@13a80b1b049f28866c3db110df5c662e8e74dede`
- `Telegram/SourceFiles/mtproto/config_loader.cpp@5d07404ba07223016c6728b986dcba10628d7194`
- `Telegram/SourceFiles/core/core_settings_proxy.h@1f2ec5c516e03ae49e80c15bb6083bffa7ec6d99`
- `Telegram/SourceFiles/core/core_settings_proxy.cpp@18e7ab8ce60865409dbaff267ee192e272622d26`
- `Telegram/SourceFiles/mtproto/mtproto_proxy_data.h@d8deebac2359936f72901ba0eec0cbfdfb186e90`
- `Telegram/SourceFiles/mtproto/mtproto_proxy_data.cpp@f9748fcb6358dd8ce7429536ec2b999cb2a989ef`

## 3. Responsibility A: external proxy mutation

`external_control.cpp` is not a second proxy-state owner. Its proxy commands are an external command adapter over the same `App().settings().proxy()` instance.

Exact command behavior:

- `proxy-list` returns list index, type, host/address, port, selected bit, current mode and enabled projection.
- `proxy-add:<link>` parses through the existing proxy-link parser, rejects invalid/unsupported/incorrect-secret values, deduplicates by exact proxy value, appends once, persists once, and exposes an undo that removes the same value; if the undone added value became selected, undo returns to System/no-selected through `Application::setCurrentProxy`.
- `proxy-remove:<index-or-link>` resolves either an integer list index or a parsed proxy link. Removal preserves the pre-removal selected/mode state for undo. Removing the selected proxy while mode is Enabled transitions to System/no-selected; removing the selected proxy while another mode is active clears selected without changing that mode. Undo reinserts at the original list index and restores selected/mode when necessary.
- `proxy-use:<index-or-link>` selects the resolved list member and forces Enabled mode.
- `proxy-next` errors on an empty list; otherwise it selects `(current_index + 1) % count`, where an unlisted/empty current selection yields index -1 and therefore selects element 0.
- `proxy-toggle` enables the existing selected proxy, or the last list entry when selected is empty; it errors only when both selected is empty and list is empty. When already enabled it retains selected but changes mode to Disabled.
- each state-changing command persists through `Local::writeSettings()`; undo closures perform the inverse mutation through the same owner and persist again.

Ownership rule: an external-control transport/socket may authenticate, parse and authorize the command, but every proxy state mutation must be a typed command into the one canonical Fabushi Settings/application owner. The external-control adapter must never persist or cache a second proxy list/selection truth.

Current Fabushi disposition: blocked/no-production-delta. The desktop checkpoint has no canonical product Settings/application owner and no shipping external local-automation proxy command surface. Implementing these mutations in telegram-network, provider-local SQLite, AppHost `ui_state`, or a new Telegram settings store would violate existing-owner-first.

## 4. Responsibility B: `useProxyForCalls`

The call-site gate is:

`useProxyForCalls && isEnabled && selected.supportsCalls() && !selected.host.isEmpty()`

Only then would the call descriptor receive host/port/login/password from a selected SOCKS5 proxy.

For this fixed U, however, `MTP::ProxyData::supportsCalls()` is exactly:

`return false; // (type == Type::Socks5);`

Therefore the descriptor-proxy assignment branch is unreachable for every persisted `useProxyForCalls` value and every selected proxy. The durable boolean remains part of SettingsProxy state for compatibility, but its non-UI call consumer has no runtime effect in this U.

Current Fabushi disposition: no-production-delta. No call-proxy injector must be added merely because the setting exists. A future upstream rebaseline that makes `supportsCalls` reachable is a new source fact and must reopen this responsibility rather than silently enabling SOCKS5 call routing.

## 5. Responsibility C: MTProto selected-proxy consumption

`MTP::Instance::Private` holds a reference to the same `Core::SettingsProxy`; it does not own selection.

Three exact consumer behaviors are isolated:

### 5.1 Proxy-enabled projection into config acquisition

- construction subscribes to `connectionTypeChanges()`;
- when a `ConfigLoader` exists, a connection-type event updates only `ConfigLoader::_proxyEnabled`;
- initial `requestConfig()` passes `_proxySettings.isEnabled()` into `ConfigLoader`;
- when `_proxyEnabled` is true, `ConfigLoader::refreshSpecialLoader()` tears down the special loader and `sendSpecialRequest()` refuses special-endpoint work.

This is a boolean projection from Settings into MTProto config acquisition. It does not make ConfigLoader a settings owner.

### 5.2 Resolved-IP cache projection

`applyDomainIps(host, ips, expireAt)` updates matching proxy values whose `tryCustomResolve()` is true. It updates both list entries and a copy of selected. If the selected copy changes while proxy mode is enabled, it writes that selected value back through `SettingsProxy::setSelected` and refreshes all live session options.

The `resolvedIPs` and `resolvedExpireAt` fields are not serialized by `SettingsProxy::serialize()`; they are transient connection observations attached to proxy values, not durable user settings.

### 5.3 Good-IP promotion projection

`setGoodProxyDomain(host, ip)` moves a known non-first resolved IP to the front for matching list entries and selected. If the enabled selected value changes, it writes selected back through the Settings owner and requests `refreshGlobalProxy()`.

Ownership rule: DNS observations and transport refresh belong to the existing network/runtime owner; durable proxy list/mode/selection remains Settings-owned. Any writeback of transient resolution order must go through the canonical Settings command/event boundary without turning the network layer into durable settings truth.

Current Fabushi disposition: blocked/no-production-delta. The current desktop `telegram-network@241cd8ec792070a8de8af9ec6ca5b317e34376d1` and `telegram-runtime@10031c5261bb0810ad2df2fca3de2e2fe4f65301` expose no proxy/config-loader/custom-resolution consumer, and the tracked desktop tree has no shipping call/proxy consumer. Adding only a cache reducer or boolean flag would be dead production code until a real selectable proxy carrier/config-acquisition path exists.

## 6. Existing-owner resolution

Accepted future split:

- canonical Fabushi Settings/application owner: durable proxy state plus external mutation commands;
- existing telegram-network/runtime owner: selected-proxy transport projection, resolution observations, session refresh and config-acquisition effects when a shipping proxy path exists;
- existing CallSession/call runtime owner: call descriptor projection only if the fixed source makes `supportsCalls` reachable;
- external-control adapter: authorization/command parsing only.

Rejected owners:

- AppHost `feature.persistence/ui_state`: UI/client persistence, not product Settings truth;
- telegram-storage: provider-private auth/transport persistence;
- fabushi-account-service: account/session owner;
- telegram-network as durable Settings owner;
- a new TelegramProxySettings/TelegramExternalControl/TelegramCallProxy state root.

## 7. Authored contracts (not executed)

Contract set `CONTRACT-TDRP-PROXY-NON-UI-CONSUMERS-001`:

- external list projection preserves list order and selected equality;
- external add deduplicates exact values;
- external remove selected+Enabled -> empty selected + System;
- external remove selected+non-Enabled -> empty selected + unchanged mode;
- external remove undo restores original list index and selected/mode;
- external use forces Enabled;
- external next on current=-1 selects index 0 and wraps modulo count;
- external toggle empty+empty errors; disabled/system with selected enables selected; disabled/system with no selected uses last list entry; enabled -> Disabled retaining selected;
- every external state mutation and undo targets one canonical Settings owner and one persistence path;
- `useProxyForCalls=true` still yields no descriptor proxy because fixed-U `supportsCalls()==false`;
- no SOCKS5 call proxy is introduced in this U;
- initial and changed proxy-enabled state is projected into config acquisition only;
- enabled proxy suppresses special-config acquisition; disabled/system does not by this flag alone;
- resolution cache is transient and omitted from durable proxy serialization;
- selected resolution-cache update refreshes session options only when proxy mode is enabled;
- good-IP promotion writes selected back and requests global proxy refresh only when enabled;
- network/config/call/external adapters never become durable proxy settings owners;
- no executable contract, build, validator, dependency resolution, CI, package or runtime acceptance is run in Stage A.

## 8. Acceptance / disposition

- AC-1: fixed-U source identities for all three non-UI consumers are recorded.
- AC-2: external mutation semantics are separated from the external-control transport and from durable Settings ownership.
- AC-3: the fixed-U call-proxy branch is explicitly classified unreachable because `supportsCalls()==false`.
- AC-4: MTProto proxy-enabled/config-loader and resolved-IP/session-refresh projections are separated from durable settings and broader carrier scheduling.
- AC-5: current Fabushi owner inventory is checked; no dead proxy/call/config-loader production API is added.
- AC-6: all three responsibilities are recorded blocked/no-production-delta with executable validation not-run and accepted=false.
- AC-7: the unique desktop ledger/P0/source-closure/evidence are synchronized through CAS.
- AC-8: Stage-A zero-CI/no-execution constraints remain intact.

## 9. Current checkpoints at discovery start

- desktop: `bhrumom/fabushi-desktop checkpoint/tdrp-stage-a-msg-mutations-7457eb1c-r1@9c92009815d2a82f6a3bd5f3b07888c4233e299f`
- platform-core: `bhrumom/fabushi-platform-core checkpoint/tdrp-telegram-auth-lifecycle-7457eb1c-r16@1cff6f7176de9d879576e0630503626636240622`

No GitHub Actions/status/check query or trigger, build, typecheck, lint, schema validator, generator, dependency resolution, lock regeneration, unit/contract/integration/E2E execution, package, deployment, installation or runtime acceptance is part of this source-review closure.
