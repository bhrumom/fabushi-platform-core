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


## 11. Final source-level compliance record

Desktop bookkeeping has now been written through the existing single migration ledger line:

- desktop bookkeeping commit: `a23452a00e9c4d395f3f97bf74ec27fd82747fb2`
- parity-ledger blob: `ca7fb45076804a0905d48ad1e3605e922310f33a`
- P0 source-closure/ledger blob: `85825d4c00857ad76abc36998e087c99afacd98f`
- source-closure blob: `c3a83028db52a1ffe25206b72512699000f2e57c`
- round114 evidence blob: `a7e318f8aa33e81fb953fb3d143cd4db43ef9588`
- coverage: `source_entries_total=16202, responsibilities_total=116, unknown=16172, unread=16145, omitted=0, unmapped_responsibilities=0`

The desktop ledger contains exactly three new responsibilities for this boundary:

- `TDRP-R9-PROXY-EXTERNAL-MUTATION-CONSUMER-001`
- `TDRP-R9-PROXY-CALL-CONSUMER-001`
- `TDRP-R9-PROXY-MTPROTO-CONSUMER-001`

All three are `mapped / pending / validation_status=not-run / accepted=false`. This is intentional: source semantics and owner boundaries are closed for this slice, but production implementation remains blocked by the missing canonical Fabushi Settings/application owner and, for MTProto runtime consumption, the absence of a shipping selectable proxy/config-acquisition consumer. The call-proxy consumer specifically requires no production delta because fixed-U `supportsCalls()` is unconditionally false.

Final source-review disposition:

| Requirement / AC | Status | Evidence / reason |
| --- | --- | --- |
| R1-R8 | passed | Fixed-U source and current-owner review recorded in this spec and round114 evidence. |
| R9 production implementation | blocked | No canonical product Settings owner; no shipping selected-proxy/config-loader carrier consumer; fixed-U call proxy branch is unreachable. |
| R10 executable validation | not-run | Stage A deliberately prohibits executable validation. |
| AC-1-AC-5 | passed | Exact source semantics and owner split are recorded without a parallel state system. |
| AC-6 | passed | Three unique desktop ledger rows exist at the bookkeeping identities above. |
| AC-7 | passed | Desktop P0/source-closure/evidence are synchronized and remotely persisted. |
| AC-8 | passed | Stage-A zero-CI/no-execution constraint was maintained. |

No production source blob changed in platform-core or desktop for this boundary. No build, test, validator, dependency resolution, Actions/status/check query, package, deployment, installation or runtime acceptance was performed.


## 12. Round 100 implementation amendment — selected-proxy carrier projection

Status: Stage A implementation authorized; executable validation remains not-run.

The blocker recorded above has changed. Desktop now has one canonical source-neutral ProductSettingsOwner v2 and one unified Settings UI/edit path. The carrier consumer may therefore be implemented without creating a second settings owner, provided the runtime receives only a transient projection after ProductSettingsOwner persistence.

### 12.1 Source facts added in this round

The fixed-U selected-proxy chain is additionally pinned to:

- `Telegram/SourceFiles/core/application.cpp@7172206f5c1e540e19c90798e9a9e5721276cc53`: `Application::setCurrentProxy` performs selected+mode mutation, refreshes the global proxy projection, emits `proxyChanges`, then emits `connectionTypeChanges`.
- `Telegram/SourceFiles/main/main_account.cpp@2721c7ac12effe454d398610d26d638fadaa1f68`: proxy changes restart MTProto sessions; a changed MTProto/Web host+port also reinitializes the main connection.
- `Telegram/SourceFiles/mtproto/session.cpp@40d7048c3057c7c5523b786048670524c8bebe4c`: `Session::refreshOptions` projects only the enabled selected proxy plus IPv6 preference into live session options.
- `Telegram/SourceFiles/mtproto/session_private.cpp@13cbc76ec723386027b5b4dcac79999b791c4614`: MTProto/Web selection forces the TCP path and publishes the original proxy host+port in `inputClientProxy`.
- `Telegram/SourceFiles/mtproto/connection_tcp.cpp@d721a9058e1885d2964ded55bd4e4c7c2ce9e381`: MTProto selection connects to the proxy host+port and uses the selected proxy secret while retaining the protocol DC id; Web selection has a separate WebProxySocket carrier.
- `Telegram/SourceFiles/mtproto/connection_abstract.cpp@3baabf3f72c7c7edba3151814681fbf167a5ca0a` and `connection_resolving.cpp@d319e632655bbfe28956c8ced981a5cdf0299d1d`: custom resolution wraps SOCKS5/MTProto hostnames, races direct no-proxy DoH requests, tries resolved IPs, and promotes a successful IP only in transient resolution order.
- `Telegram/SourceFiles/mtproto/mtproto_proxy_data.cpp@f9748fcb6358dd8ce7429536ec2b999cb2a989ef`: `tryCustomResolve()` is true only for SOCKS5/MTProto non-IP hosts; `resolvedIPs` / `resolvedExpireAt` are not durable proxy fields.

### 12.2 Target ownership and control flow

The implementation contract is:

`ProductSettingsOwner persist -> transient selected-proxy projection -> existing RuntimeClient -> existing telegram-network carrier`.

Requirements for this projection:

1. ProductSettingsOwner remains the only durable owner of mode, selected/list, IPv6 preference, credentials and rotation settings.
2. RuntimeClient may retain only a transient carrier projection and transient DNS observations. It must not write a second settings file or provider-private settings record.
3. An applied projection change invalidates/cancels live network sessions/owned requests so the next session is opened from the new projection while reusable auth material remains owned by the existing SessionOwner.
4. System/Disabled projects direct transport. Enabled without a selected proxy fails closed; it must never silently fall back to direct transport.
5. The existing native carrier in this round implements the MTProto-proxy route: connect to the selected proxy address, use the selected MTProto secret, retain the destination protocol DC id, and use the source-defined connection-start/TLS-camouflage machinery already owned by telegram-network.
6. SOCKS5, HTTP and Web selections must fail closed at this carrier boundary until their already-separated carrier responsibilities are implemented. In particular, an enabled unsupported proxy must not leak traffic through a direct Telegram connection.
7. MTProto proxy hostnames use the already-owned direct no-proxy DomainResolver request/response normalization, 800 ms staggered race and source-bounded TTL. Resolved IP order and expiry are transient only.
8. The good resolved IP may be promoted in the transient runtime/network projection after a successful connection. No durable Settings write is required because fixed-U serializes neither resolved IPs nor expiry.
9. IPv6 preference is projected into the existing NetworkConfig from canonical Settings; it does not become independently persisted by RuntimeClient.
10. No proxy secret may appear in logs, status payloads or error strings.

### 12.3 Publication contract

Desktop publication must occur after ProductSettingsOwner persistence for every mutation that can change the active carrier projection: mode, selected proxy or IPv6 preference. Restored product Settings must also be projected into the existing process-lifetime TelegramClientLease before that lease is used for network work. The existing `proxyEnabled -> telegram.refreshSpecialConfig` suppression publication remains part of the same post-persistence publication and is not replaced by a second settings owner.

### 12.4 Authored contracts for this amendment

The implementation must author, but Stage A must not execute, contracts proving:

- System/Disabled yields direct NetworkConfig and Enabled+missing-selected fails closed.
- an enabled MTProto projection maps host/port/secret/IPv6 to the existing carrier without persisting it;
- unsupported enabled SOCKS5/HTTP/Web projections fail closed rather than direct-connect;
- a hostname MTProto proxy uses normalized DomainResolver attempts, accepts source-bounded TTL and filters response data to matching IP families;
- a successful resolved proxy IP is promoted only in transient resolution order;
- applying a changed projection invalidates live sessions/owned requests while preserving reusable auth material and persisted auth-key state;
- startup restoration publishes the current ProductSettingsOwner projection before normal host use;
- mode/selected/IPv6 mutations publish after persistence;
- proxy secrets are absent from response/status/error projections;
- the existing special-config suppression path still receives the same authoritative enabled boolean;
- all new APIs remain source-neutral carrier/settings projections and no Telegram-specific settings/store/runtime owner is introduced.

### 12.5 Round 100 acceptance

This responsibility may advance to `implemented / validation_status=not-run / accepted=false` only when the platform network/runtime implementation, desktop mirrored production sources, ProductSettingsOwner publication, TelegramClientLease adapter and the authored contracts are all committed into the existing shipping composition. Stage B executable validation remains the only validation blocker for the implemented MTProto selected-proxy/custom-resolution slice; unsupported proxy carrier types remain explicitly fail-closed under their separate carrier responsibilities.


## 13. Round 100 source-level implementation record

The Stage-A implementation described in section 12 is now present in the existing owners. No executable validation was run.

Platform implementation commit:
- `2a4e9542f8b102eeaac8cc28f123e1a62648235d`
- `native/telegram-protocol/src/web_proxy.rs@98e65d426242f9ff1ca0a3a2213dae1e41eca824`: shared MTProto secret decoding accepts fixed-U 16-byte, 0xdd and 0xee forms while the WebProxy parser continues to reject 0xee.
- `native/telegram-network/src/domain_resolver.rs@ef6b36c26257ee375df0a690e63dae814656e545`: transient hostname cache, direct no-proxy normalized DoH attempts, source-bounded TTL and successful-IP promotion.
- `native/telegram-network/src/lib.rs@cf1247b3ec2486ce5e05f5a08ceb30b1e460e1db`: source-neutral NetworkProxyProjection, MTProto proxy carrier connection, unsupported enabled carrier fail-closed behavior and proxy-aware auth-key resume/bootstrap.
- `native/telegram-runtime/src/runtime.rs@b605b13c110b545d0c703b7fae1cc27d3d10afab`: transient projection command, session/request invalidation on projection change, reusable auth-material preservation and proxy-aware session opening.

Desktop production composition commit:
- `5d816f224c21019f75725b56b6f8c759871f5295`
- provider protocol/network/runtime files mirror the platform implementation blobs above;
- `mahayana-telegram/src/lib.rs@8df8e7286f161452336727d7e5750e45b67c413f` adds only a typed carrier projection adapter on the existing process-lifetime TelegramClientLease;
- `mahayana-app-host/src/lib.rs@aa10fcfde5f47fc4b56223ff466cd9f128300b33` derives the carrier projection from the canonical ProductSettingsOwner v2, publishes restored settings at startup, and republishes mode/selected/IPv6 changes only after authoritative persistence.

Ownership remains unchanged: ProductSettingsOwner is the only durable proxy settings owner; RuntimeClient and telegram-network hold transient connection state only. No Telegram-specific settings/store/runtime root was added. Enabled SOCKS5/HTTP/Web selections fail closed under their separately open carrier responsibilities instead of falling back to direct Telegram transport.

Authored contracts live next to the production owners and cover fail-closed unsupported carriers, transient resolver promotion, source-neutral carrier DTOs, publication mutation surface, session invalidation and secret-free response projections. They are intentionally not executed in Stage A.

Round-100 source-level disposition for `TDRP-R9-PROXY-MTPROTO-CONSUMER-001`: `implemented / validation_status=not-run / accepted=false`. The implemented slice is MTProto selected-proxy/custom-resolution consumption. Stage-B executable validation remains open; SOCKS5/HTTP/Web carrier implementation remains tracked by their separate responsibilities.


## 14. Round 100 startup publication correction

Desktop manual source review after the initial production composition found that startup restore must publish only the transient carrier projection. Calling `refreshSpecialConfig(false)` during AppHost construction can require authorization phone context and is not required to restore the carrier route.

Desktop correction commit: `2f0798e8709b3a3ea6b200f092060db6c1dccb62`; corrected AppHost blob: `1fe55278a834b828b6ffdd8e2ca933fd2eaa3ef9`.

The final publication split is therefore:
- startup restore: ProductSettingsOwner snapshot -> `TelegramClientLease::apply_network_proxy` only, before normal Telegram host use;
- persisted mode/selected/IPv6 mutations: carrier projection publication, and the existing mode-derived `refreshSpecialConfig` publication where the settings mutation surface already performs it.

This preserves the source-neutral carrier restore without introducing an authorization-context dependency into startup. The correction was source-reviewed only; no executable validation was run.
