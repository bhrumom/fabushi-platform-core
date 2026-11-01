# Fabushi product Settings owner — Specification / ADR

Status: active  
Owner: canonical Fabushi application host Settings owner  
Decision date: 2026-11-01  
Task: 7457eb1c-11a3-4626-8563-a3e495872536 round 97

## 1. Decision

Fabushi has no existing shipping product Settings state owner in the reviewed checkpoints. The existing `feature.persistence.*` surface persists arbitrary UI/client state in `ui_state`; Telegram storage is provider-private auth/transport state; account-service owns account/session state; telegram-network/runtime owns transport execution. None may become the durable source of product preferences.

Create one minimal source-neutral Settings owner at the shipping application-host composition boundary. It is the single authority for product settings state and persistence. The first absorbed setting is the network proxy-enabled preference required by fixed-U Telegram special-config behavior. The owner is deliberately product-generic: it is not named for Telegram, stores no Telegram API credentials, proxy endpoint secrets, socket handles, or provider runtime state, and is extensible for future Settings sections.

## 2. Scope

This ADR/spec establishes:
- one typed `ProductSettingsState` with a `network.proxyEnabled` field;
- one `ProductSettingsOwner` lifecycle under the shipping application host;
- durable source-neutral persistence separate from `ui_state` and provider-private stores;
- top-level typed Settings command/snapshot host surfaces;
- authoritative publication of the current `proxyEnabled` value to the already-existing `telegram.refreshSpecialConfig` RuntimeClient boundary whenever the setting is commanded;
- repeat publication even when the requested value equals the stored value, so a failed downstream publication can be retried without manufacturing another state transition.

No Telegram-specific Settings store, second Settings root, WebProxy carrier, proxy endpoint list/selection model, rotation scheduler, or UI clone is introduced in this round.

## 3. Fixed-U source contract

Source: `telegramdesktop/tdesktop@f985723b552b30fe1c17d377d57f44e61a15a8c4`.

Relevant facts:
- `Core::SettingsProxy::setSettings` changes the authoritative mode value only.
- `Core::SettingsProxy::connectionTypeChangesNotify` publishes a connection-type event.
- `Application::setCurrentProxy` updates selected+mode, refreshes the global projection, emits proxy change, emits connection-type change, and notifies rotation state.
- `connection_box.cpp` mutates the single application settings object and persists through the application settings path; it is not a second owner.
- MTProto consumes the enabled projection for special-config suppression. The already-implemented Fabushi RuntimeClient command requires explicit boolean `proxyEnabled`.

This round ports only the authoritative boolean publication needed by the current RuntimeClient boundary. Remaining selected-proxy/custom-resolution consumers stay separate.

## 4. Existing-owner-first review

Rejected as canonical Settings owners:
- `mahayana-app-host::feature.persistence/ui_state`: generic UI/client key-value persistence, no typed product Settings ownership.
- `telegram-storage`: provider-private authorization/transport persistence.
- `fabushi-account-service`: account/session owner.
- `telegram-network` / `telegram-runtime`: transport/runtime consumers, not durable product preference owners.
- a new Telegram-specific settings crate/store: prohibited duplicate product truth.

Selected composition: the existing shipping `AppHost` is the application lifecycle/composition owner and therefore hosts one source-neutral `ProductSettingsOwner`. This is an extension of an existing owner, not a parallel application.

## 5. State and commands

Initial schema:

```text
ProductSettingsState {
  version: 1,
  network: {
    proxyEnabled: bool
  }
}
```

Defaults:
- version = 1
- network.proxyEnabled = false

Commands:
- `settings.snapshot`
- `settings.setNetworkProxyEnabled { proxyEnabled: bool }`

The mutation is serialized by the Settings owner. Persistence succeeds before the authoritative projection is published. Reissuing the same command must republish the authoritative value.

## 6. Persistence

The owner persists a single versioned product Settings document at the application-host settings path. Persistence is not stored in `ui_state` and not in Telegram's encrypted provider database.

Requirements:
- create parent directories before first write;
- serialize only typed source-neutral state;
- write a replacement temporary file then rename into the canonical path;
- malformed/unreadable existing Settings data fails closed instead of silently replacing it;
- no secrets are logged;
- future schema changes must be versioned/migrated in this owner.

## 7. Runtime publication

The application host publishes through the existing embedded Telegram client lease, which forwards:

```json
{"@type":"telegram.refreshSpecialConfig","proxyEnabled":<authoritative bool>}
```

The RuntimeClient remains a consumer only and does not persist the setting. `proxyEnabled=true` keeps the already-authored zero-work suppression behavior. `proxyEnabled=false` permits the existing special-config refresh path. If no embedded Telegram client is present, the product setting is still persisted; publication is reported as unavailable rather than creating a second owner.

## 8. Failure and recovery

- Invalid/missing `proxyEnabled`: reject without mutation.
- Settings persistence failure: reject; do not publish a value that is not authoritative.
- Runtime publication failure after persistence: return failure while retaining the persisted setting. The same command is retry-safe because publication is repeated even when the value is unchanged.
- Malformed persisted Settings: fail AppHost initialization closed.
- Provider unavailable: do not move Settings authority into provider state.

## 9. Stage-A contracts

Author but do not run:
- default Settings state is proxyEnabled=false;
- mutation persists and reloads the same value;
- same-value command is publication-idempotent/retryable;
- malformed persisted Settings fails closed;
- the RuntimeClient request contains explicit boolean `proxyEnabled`;
- RuntimeClient owns no durable proxy Settings field;
- no `TelegramSettingsStore` or provider-local product preference database is introduced.

Validation status remains `not-run`; acceptance remains false.

## 10. Definition of Done for this round

- durable spec/ADR committed before production implementation;
- one source-neutral Settings owner exists at AppHost composition;
- authoritative `proxyEnabled` command persists through that owner;
- publication reaches the existing `telegram.refreshSpecialConfig` boundary through the existing Telegram client lease;
- contracts are authored but not executed;
- desktop ledger/P0/source-closure/evidence record exact commits/blobs and Stage-A status;
- no CI/build/test/lint/package/deploy execution occurs.


## 11. Round 97 implementation / source-compliance evidence

Source-level implementation review against desktop commit `b1b5dd027497c39a355e9e64e64de0b6c190fd66` records:

- shipping AppHost blob: `5b74723e12884ea218e1f9de634b88d4357b2099`;
- shipping Mahayana Telegram adapter blob: `e4410652fe500144d93ff227b10d149d2c0fc2c7`;
- mirrored owner spec blob: `2b807499beecd2d912de82aa36a27d24291d8521`;
- `ProductSettingsOwner` is composed into the existing `AppHost`, owns versioned `network.proxyEnabled`, and persists `feature-host/product-settings.json` via temporary-file replacement;
- the existing `feature.persistence.* / ui_state` path is unchanged and is not used as Settings truth;
- `settings.setNetworkProxyEnabled` requires an explicit boolean, persists canonical state first, then publishes the authoritative value through the existing `TelegramClientLease`;
- `TelegramClientLease::refresh_special_config` forwards exactly `telegram.refreshSpecialConfig` plus explicit boolean `proxyEnabled` into the already-existing RuntimeClient command boundary;
- no proxy Settings field was added to RuntimeClient and no Telegram-specific Settings store/root was introduced;
- source-authored contracts cover persistence/reopen, malformed-data fail-closed, source-neutral host method names, and exact boolean RuntimeClient request shape.

Source-level requirements in sections 1-8 and Stage-A implementation portions of section 10 are satisfied. Executable validation is deliberately not performed in Stage A, so validation remains `not-run`, acceptance remains false, and no release/verification claim is made. The broader fixed-U proxy list/selected/mode/rotation migration remains outside this minimal owner slice and continues under `telegram-proxy-settings-state-persistence.md`.


## 12. Round 98 typed network proxy state extension (spec-first)

Round 98 extends the same canonical, source-neutral ProductSettingsOwner; it does not create a Telegram-specific settings store, provider-local preference database, second Settings root, or carrier owner.

The canonical persistence schema advances from version 1 to version 2. Version 2 replaces the lossy stored network.proxyEnabled boolean with one typed network.proxy object:

```text
ProductSettingsState v2 {
  version: 2,
  network: {
    proxy: {
      mode: System | Enabled | Disabled,
      selected: ProxyEndpoint?,
      list: ProxyEndpoint[],
      tryIpv6: bool,
      useProxyForCalls: bool,
      checkIpWarningShown: bool,
      rotation: {
        enabled: bool,
        timeoutSeconds: positive integer,
        preferredIndices: integer[]
      }
    }
  }
}

ProxyEndpoint {
  type: Socks5 | Http | Mtproto | Web,
  host: string,
  port: uint32,
  user: string,
  password: string
}
```

Resolved IPs, DNS expiry, sockets, WebView handles, checker results, timers, and carrier state are explicitly excluded. They remain transient runtime concerns. ProxyEndpoint retains user/password because fixed-U persistent proxy records use those fields for authentication and, for MTProto/Web forms, source-defined secret/base-path encodings; the Settings owner must never log those values.

Canonical defaults preserve fixed-U SettingsProxy semantics: mode=System, selected=None, list=[], useProxyForCalls=false, checkIpWarningShown=false, rotation.enabled=false, rotation.timeoutSeconds=10, rotation.preferredIndices=[]; tryIpv6=false on Windows and true on other supported desktop platforms.

The v1 -> v2 in-place migration is authoritative for the existing Fabushi product-settings.json path: proxyEnabled=true maps to mode=Enabled; proxyEnabled=false maps to mode=System because v1 could only distinguish the Enabled gate from the non-Enabled gate and System is the fixed-U/default neutral mode. All other v2 fields receive canonical defaults. Migration is persisted back through the same ProductSettingsOwner path before the owner becomes available. Malformed or unsupported versions remain fail-closed.

proxyEnabled is no longer separately persisted. It is derived exactly as mode == Enabled and is published only to the already-existing telegram.refreshSpecialConfig consumer boundary. System and Disabled both project false. No selected/list endpoint is sent to an unimplemented carrier.

The source-neutral mutation contract is settings.mutateNetworkProxy with a typed operation. Supported canonical operations are setMode, setSelected, setSelectedAndMode, setList, add, insert, remove, replace, setTryIpv6, setUseProxyForCalls, setCheckIpWarningShown, setRotationEnabled, setRotationTimeout, setRotationPreferredIndices, and promoteRotationPreferredIndex. setSelectedAndMode exists so selection+mode can be one domain mutation rather than two externally observable writes. The compatibility settings.setNetworkProxyEnabled method remains source-neutral and is interpreted as a projection command: true selects Enabled; false moves Enabled back to System but preserves an already-System or already-Disabled mode; it republishes the derived boolean even on a no-op retry.

Mutation invariants are the fixed-U SettingsProxy invariants: non-positive rotation timeout normalizes to 10; preferred indices drop negative/out-of-range/duplicate values preserving first occurrence; setList clears preferred indices; insert clamps to [0,list.len] and increments preferred indices at/after insertion; remove decrements indices after the removed element and drops the removed index; replace preserves index ordering; promotion is invalid/no-op out of range, idempotent when already first, otherwise moves/inserts the index to the front.

Stage-A contracts must be authored for v1 migration, defaults, all list/preferred/timeout invariants, mode-derived proxyEnabled publication, same-value retry, and fail-closed malformed/unsupported state. They are not executed in Stage A.
