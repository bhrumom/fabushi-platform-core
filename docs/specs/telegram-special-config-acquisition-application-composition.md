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
