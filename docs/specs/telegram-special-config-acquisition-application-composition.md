# Telegram Special-Config Acquisition → Existing Runtime Application — Specification

Status: Stage A implementation authorized / validation not run
Fixed upstream: telegramdesktop/tdesktop@f985723b552b30fe1c17d377d57f44e61a15a8c4
Existing owners only: native/telegram-network + native/telegram-protocol + native/telegram-runtime
Task: 7457eb1c-11a3-4626-8563-a3e495872536 round 95

## 1. Source anchors

- Telegram/SourceFiles/mtproto/special_config_request.cpp@fdb65506b0a886bc2daeccb61090bc44be7e2d88
- Telegram/SourceFiles/mtproto/config_loader.cpp@5d07404ba07223016c6728b986dcba10628d7194
- Telegram/SourceFiles/mtproto/mtproto_config.cpp at fixed U: production TXT domain apv3.stel.com; test TXT domain tapv3.stel.com.
- Existing request normalization: native/telegram-network/src/request_normalization.rs@2f3ea627054779296102f13c70be17a3655ddcb9
- Existing direct carrier/race: native/telegram-network/src/direct_https.rs@0c7444d23fdee0aa135f12fd55a50ec3fe42c690
- Existing response normalization: native/telegram-protocol/src/response_normalization.rs@8d2d70ec883ea8969be68b280e994e2bb35baf71
- Existing special-config decode/select: native/telegram-protocol/src/special_config.rs@03ea4149703942befe59132a2019bef71e92938b
- Existing endpoint application: native/telegram-runtime/src/runtime.rs@b502e32e821433a14cc47861e17c10e106fd5af8, apply_resolved_special_config_endpoint.

## 2. Fixed-U acquisition contract

Config-bearing SpecialConfigRequest seeds Google dns.google.com and Mozilla mozilla.cloudflare-dns.com. Outside test mode it also seeds Firestore directly plus one Firestore camouflage attempt for each DnsDomains entry. Google/Mozilla are shuffled only within their two-element group; Firestore attempts are shuffled only within the Firestore group. The resulting groups remain ordered DoH first, Firestore second. Dispatch remains 800 ms staggered through the already-owned DirectHttpsRace SpecialConfig settlement policy; accepting/finalizing one reply does not fence later scheduled attempts.

Each finished reply exposes a successful HTTP Date header to the existing HTTP-time owner before body decoding. A readable body remains eligible for normalization even when HTTP status is not successful. Google/Mozilla bodies use TXT-only concatenation. Firestore bodies use fields.data.stringValue extraction. Empty/invalid normalized payloads do not settle the aggregate acquisition.

Decoded help.configSimple is filtered by current HTTP-adjusted time and the current account phone-prefix rules. Every decoded endpoint is delivered to the endpoint queue; a single decodable response does not stop the remaining HTTP attempt schedule.

## 3. ConfigLoader endpoint-queue contract

The runtime composition shall preserve the ConfigLoader queue responsibilities without creating another runtime:

- pending and tried endpoints are deduplicated;
- an endpoint already pending or tried is not enqueued again;
- pending endpoint selection is randomized;
- each selected endpoint is removed from pending and added to tried before application;
- application calls the existing apply_resolved_special_config_endpoint only;
- that existing function remains the sole owner of temporary 1000+d routing, per-real-DC auth-material reuse/creation, bare help.getConfig, 6000 ms request ownership, temporary-session teardown, and dc_options-only replacement;
- failure of one queued endpoint does not prevent trying the remaining queued endpoints;
- no TelegramRuntime/Provider/HTTP subsystem is added.

The synchronous Fabushi runtime exposes this composition through its existing RuntimeClient command surface. It does not invent a second state owner. Fixed-U proxy suppression remains a higher settings/composition concern until the canonical proxy state owner is connected to this runtime.

## 4. Owner split

native/telegram-network owns request seeding/shuffle entropy, request normalization, direct NoProxy HTTPS/Web-PKI transport, staggered request ownership, and HTTP-time mutation storage.

native/telegram-protocol owns Date parsing, response body normalization, help.configSimple cryptographic decode, phone/expiry selection, and SpecialConfigEndpoint values.

native/telegram-runtime owns the account-scoped orchestration and endpoint queue and calls the existing apply_resolved_special_config_endpoint.

## 5. Stage-A contracts

Author but do not run contracts for deterministic injected-entropy seeding/shuffle, test-mode Firestore suppression, DoH-before-Firestore group ordering, endpoint pending/tried dedupe, randomized dequeue by supplied index, Date-before-decode composition, continued processing after malformed/failed replies, continued queued-endpoint application after one endpoint failure, and dc_options-only application through the existing RuntimeClient.

No build, lint, typecheck, dependency resolution, unit/contract/integration/E2E, packaging, deployment, installation, Actions/status/check query, or runtime acceptance is authorized in Stage A.

## 6. Acceptance state

Source implementation may advance to implemented / validation_status=not-run / accepted=false only after the real RuntimeClient command path composes the existing network/protocol owners into apply_resolved_special_config_endpoint. Stage-B executable evidence remains required for verified/accepted.

## 7. Round 95 source-compliance record

Fixed-U reread:
- special_config_request.cpp blob fdb65506b0a886bc2daeccb61090bc44be7e2d88
- config_loader.cpp blob 5d07404ba07223016c6728b986dcba10628d7194
- mtproto_config.cpp blob e8fc605ca197d2bc82a9f73da18eeb945967a99f

Implementation sequence:
- spec-first authorization: bab99fd54de1cf98dd781f335dad666b3fa2b3ab
- attempt seeding/shuffle owner: 2224fd041f99e3153be80c8b667270fbbd6ce3c5
- RuntimeClient acquisition/application composition: 87bb86ae509ca09c46d1b9e26f5177f81bb54830
- endpoint-queue timing/source-review correction: 602b67b2c2c4b11f197ec968e8bc964e89af7695

Final implementation blobs before this documentation commit:
- telegram-network special_config_acquisition.rs: 9c8ef0ee0176a78eaa98266b5f92db1757b5db7c
- telegram-network lib.rs: b073f01ab05e692b46afa4fc931cd3e8dd0893cf
- telegram-runtime runtime.rs: df8ed7c3cebed632f6591c5809c28753ede9bba0
- telegram-runtime lib.rs: 9acba52c16de3085a3dbfa61dc3e9f6ba5cdb689

Source review confirms that the shipping RuntimeClient command path now composes the existing request-normalization, direct HTTPS race, HTTP Date/time owner, response-normalization, special-config decode/select, endpoint queue, and apply_resolved_special_config_endpoint owners. Endpoint queue draining begins after each decoded response while the independent HTTP stagger remains live, preserving the source distinction that a decodable SpecialConfigRequest reply does not fence later HTTP attempts. Pending/tried dedupe and randomized dequeue are source-owned in the existing runtime, while temporary-session/help.getConfig/dc_options application remains solely in apply_resolved_special_config_endpoint.

Authored contracts exist in the network seeding module and RuntimeClient queue module. They were not executed. No build, typecheck, lint, validator, generator, dependency resolution, lock regeneration, unit/contract/integration/E2E, package, deployment, installation, runtime acceptance, Actions query, status query, or check query was performed.

Stage-A disposition for TDRP-R9-MTPROTO-SPECIAL-CONFIG-CARRIER-APPLICATION-001: implementation_status=implemented, validation_status=not-run, accepted=false. Stage-B executable evidence remains required before verified/accepted.



## 8. Round 96 proxy-enabled suppression contract

Additional fixed-U anchors:
- Telegram/SourceFiles/mtproto/config_loader.cpp@5d07404ba07223016c6728b986dcba10628d7194: `refreshSpecialLoader()` resets the special loader when `_proxyEnabled` is true; `sendSpecialRequest()` terminates owned special MTProto work and resets the loader when the same projection is true; `setProxyEnabled()` only updates that projection.
- Telegram/SourceFiles/mtproto/mtp_instance.cpp@d25fe3b72298fab371854f80186ee8e4777ea087: `Core::SettingsProxy::connectionTypeChanges()` projects `SettingsProxy::isEnabled()` into an existing ConfigLoader, while initial ConfigLoader construction receives the same `isEnabled()` value.
- Telegram/SourceFiles/core/core_settings_proxy.cpp@18e7ab8ce60865409dbaff267ee192e272622d26: `isEnabled()` means exactly `ProxyData::Settings::Enabled`; System and Disabled do not assert this special-config suppression flag.

Existing-owner-first rule for Fabushi:
- durable proxy mode/list/selection remains a Settings/application responsibility and MUST NOT be persisted in telegram-runtime or telegram-network;
- the current checkpoint still has no canonical product Settings/application state owner, as recorded by telegram-proxy-settings-state-persistence.md and telegram-proxy-non-ui-consumers-boundary.md;
- therefore RuntimeClient accepts only a caller-supplied boolean projection on `telegram.refreshSpecialConfig`; it does not become a proxy settings owner;
- when `proxyEnabled=true`, the refresh command returns a source-equivalent suppressed report before seeding, normalizing, or dispatching any special-config HTTP attempt, and before creating any temporary special-config MTProto request;
- when `proxyEnabled=false` (including the source-equivalent System/Disabled cases), the existing acquisition/application composition is unchanged;
- the lower-level Rust function takes the same projection explicitly so no bypassing public path silently starts acquisition under an enabled proxy;
- a future canonical Settings owner may publish this boolean into the command boundary, but this round MUST NOT fabricate a second settings store, cache, or durable proxy state.

Authored-only contracts:
- enabled proxy => suppressed report, zero seeded/finished/decoded/selected/applied endpoints, no changed DC ids, no failures;
- disabled/System projection => suppression is false and the ordinary acquisition path remains eligible;
- command parsing requires a boolean `proxyEnabled` field and rejects non-boolean input;
- RuntimeClient remains settings-stateless with respect to durable proxy mode/list/selection.

Validation remains deliberately not run in Stage A.


## 9. Round 96 source-compliance record

Spec-first commit: cb9b53d2574c012fe90e03df81b8b1336859ee2b.
Production implementation commit: 1c315c155eb0ba0c49871c08547bb7e67977aa87.
Runtime implementation blob: 871ebe40ecbd284828b0c16ab64547eefeb3c3a3.

The existing `telegram.refreshSpecialConfig` command now requires the source-neutral `proxyEnabled` projection. `true` is handled before phone resolution, request seeding, request normalization, DirectHttpsRace construction, endpoint decode/application, or temporary MTProto request/session creation, and returns `SpecialConfigRefreshReport { suppressed_by_proxy: true, ...zero work... }`. The public Rust `refresh_special_config` function carries the same explicit projection, so callers cannot bypass the command boundary and accidentally start special-config acquisition while the canonical proxy mode is enabled.

No durable proxy mode/list/selection was added to RuntimeClient. The current checkpoint still lacks a canonical product Settings/application persistence owner, so this implementation deliberately stops at the typed Settings-to-runtime projection boundary rather than fabricating a Telegram settings store. The previously mapped `TDRP-R9-PROXY-MTPROTO-CONSUMER-001` therefore remains pending for its upstream Settings publication edge, while its special-config suppression consumer is now implemented in the real RuntimeClient composition.

Three contracts were authored in `native/telegram-runtime/src/runtime.rs` for direct suppression, strict boolean command parsing, and suppressed command response without phone/network work. They were not executed. No build, typecheck, lint, validator, generator, dependency resolution, lock regeneration, test, package, deployment, installation, runtime acceptance, Actions query, status query, or check query was performed.
