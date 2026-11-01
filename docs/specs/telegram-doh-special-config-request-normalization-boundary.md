# Telegram DoH / Special-Config Request Normalization Boundary — Specification

Status: active  
Owner: native/telegram-network (pure request shaping); native/telegram-network scheduler/runtime remains separately pending  
Last updated: 2026-11-01  
Related task: Fabushi migration task 7457eb1c-11a3-4626-8563-a3e495872536 round 86 / source-evidence round 117

## 1. Context

Fixed upstream remains `telegramdesktop/tdesktop@f985723b552b30fe1c17d377d57f44e61a15a8c4`.

This round rereads the request-shaping boundary in:

- `Telegram/SourceFiles/mtproto/details/mtproto_domain_resolver.cpp@eeac8f6a8e2df67967ea7b536fb165e2f8d983e9`
- `Telegram/SourceFiles/mtproto/special_config_request.cpp@fdb65506b0a886bc2daeccb61090bc44be7e2d88`

The reviewed code contains request normalization that is independent of timers, request racing, reply ownership, cancellation and temporary-DC lifecycle:

1. `DnsDomains()` exposes the four fixed camouflage domains in source order.
2. `GenerateDnsRandomPadding()` chooses a length in 13..=128 by rejecting count bytes below the source threshold, then maps random bytes through the exact 62-character source alphabet.
3. `DnsUserAgent()` returns one fixed UA string.
4. Domain-resolution Google/Mozilla attempts deterministically produce scheme/host/path/query plus Google Host camouflage or Mozilla Accept.
5. Special-config Google/Mozilla attempts deterministically produce the corresponding TXT/ANY query shapes.
6. Firestore attempts deterministically produce the direct or camouflage URL host, fixed path and optional `Host: firestore.googleapis.com`.

The later no-proxy HTTP dispatch, 800 ms staggered racing, reply lifecycle, global HTTP-time mutation, temporary-DC `help.getConfig` lifecycle and dc-options application remain scheduler/runtime concerns.

## 2. Ownership decision

The canonical `native/telegram-network` crate is the existing carrier owner and already depends on `getrandom`. Its current shipping implementation is TCP/MTProto-focused and contains no HTTP URL/header scheduler, but request shaping does not require one.

Therefore this responsibility is added as a pure module inside the existing network crate:

- semantic request components only;
- explicit entropy-injection API for source-exact padding contracts;
- production padding convenience wrapper reusing the existing `getrandom` dependency;
- no HTTP client, timer, proxy state, cache, reply object or temporary-DC state.

The response JSON codec remains in `native/telegram-protocol`; no duplicate response parser or second carrier is introduced.

## 3. Requirements

- **R1 — Fixed source identity.** Preserve the two fixed-U blobs listed above.
- **R2 — DNS domains.** Preserve exactly `google.com`, `www.google.com`, `google.ru`, `www.google.ru` in source order.
- **R3 — Padding count.** Reproduce `1 + random_byte / 2` with rejection until the result is at least 13; the accepted count is therefore 13..=128.
- **R4 — Padding alphabet.** Reproduce byte modulo the exact ASCII alphabet `abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789` for every padding character.
- **R5 — User agent.** Preserve exactly `Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/151.0.0.0 Safari/537.36`.
- **R6 — Domain resolver Google.** Produce HTTPS, endpoint host, path `/resolve`, query `name=<domain>&type=<1|28>&random_padding=<padding>`; when an attempt host alias exists produce `Host: <alias>.google.com`; do not add Mozilla Accept.
- **R7 — Domain resolver Mozilla.** Produce HTTPS, endpoint host, path `/dns-query`, the same numeric type query, `Accept: application/dns-json`, and no Google Host override.
- **R8 — Special-config Google/Mozilla.** Google uses `/resolve` and `type=ANY`; Mozilla uses `/dns-query` and `type=16`; both preserve `name=<domainString>`, `random_padding=<padding>`, UA and their source-specific Host/Accept behavior.
- **R9 — Firestore.** Direct `data=firestore, host=<empty>` normalizes to URL host `firestore.googleapis.com`; camouflage `data=<dns-domain>, host=firestore` keeps the DNS-domain URL host and adds `Host: firestore.googleapis.com`. Both use `/v1/projects/reserve-5a846/databases/(default)/documents/ipconfig/v3`.
- **R10 — Component boundary.** Return semantic scheme/host/path/query/header components instead of opening a socket or owning a reply. Query text remains source-shaped; the future HTTP carrier must apply it with URL semantics equivalent to fixed-U `QUrl::setQuery`.
- **R11 — Existing-owner-first/dependency discipline.** Reuse `native/telegram-network` and its existing `getrandom`; no manifest or lock delta and no parallel HTTP/bootstrap runtime.
- **R12 — Deferred validation.** Author internal/public contracts but do not execute build, test, lint, validator, generator, dependency resolution, CI, packaging or runtime acceptance in Stage A.
- **R13 — Scheduler split.** Keep no-proxy HTTP dispatch, staggered attempt scheduling/racing, reply cancellation/finalization, global HTTP-time mutation, temporary-DC lifecycle and dc-options application mapped/pending.

## 4. Target API

Add `native/telegram-network/src/request_normalization.rs` with source-neutral APIs:

- `DNS_DOMAINS`
- `DNS_USER_AGENT`
- `DNS_JSON_ACCEPT`
- `RequestProvider::{Google, Mozilla}`
- `NormalizedServiceRequest`
- `generate_dns_random_padding_with_entropy`
- `generate_dns_random_padding`
- `normalize_domain_resolver_request`
- `normalize_special_config_doh_request`
- `normalize_special_config_firestore_request`

`NormalizedServiceRequest` carries only scheme, URL host, path, optional query, optional Host override, optional Accept and User-Agent.

## 5. Failure and security semantics

- Entropy failure returns a typed error and does not synthesize predictable padding.
- The normalizers do not validate or rewrite the caller-supplied domain/endpoint strings beyond fixed source composition; the future carrier remains responsible for applying URL semantics.
- No payload, secret, proxy state or response body is logged.
- No network request is performed by this module.

## 6. Authored-only contracts

Write but do not run contracts for:

- exact domain list and UA;
- padding rejection boundary, 13/128 endpoints and modulo-62 alphabet mapping using injected entropy;
- Google/Mozilla A/AAAA request components;
- special-config Google ANY and Mozilla TXT request components;
- direct and camouflage Firestore host/path behavior;
- absence of scheduler/proxy/reply state from the request descriptor.

## 7. Acceptance criteria

- **AC-1:** The spec is committed before production implementation.
- **AC-2:** Pure request shaping is implemented in existing `telegram-network` with no HTTP client/scheduler.
- **AC-3:** Existing `getrandom` is reused with no Cargo/lock delta.
- **AC-4:** Internal/public contracts are authored and remain not-run.
- **AC-5:** Canonical platform and desktop request-normalization source/lib/contracts are byte-identical.
- **AC-6:** The unique ledger/P0/source-closure/evidence line records the new responsibility and reduces the carrier blocker from request construction to request dispatch/racing/application.
- **AC-7:** Carrier/application remains `mapped / pending / validation_status=not-run / accepted=false`.
- **AC-8:** Stage A makes no executable-validation or release-readiness claim.

## 8. Provenance

Behavioral provenance only; no upstream C++ is copied.

## 9. Compliance record

Spec-first boundary recorded at source-review level. Production implementation and authored-only contracts are the next step in this same round. Executable validation remains intentionally deferred.
