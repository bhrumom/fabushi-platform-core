# Telegram DoH / Special-Config Response Normalization Boundary — Specification

Status: active  
Owner: native/telegram-protocol (pure response codec); native/telegram-network remains carrier/scheduler owner  
Last updated: 2026-11-01  
Related task: Fabushi migration task 7457eb1c-11a3-4626-8563-a3e495872536 round 85 / source-evidence round 116

## 1. Context

Fixed upstream remains `telegramdesktop/tdesktop@f985723b552b30fe1c17d377d57f44e61a15a8c4`.

The prior special-config payload boundary already moved compact payload cryptographic decoding and endpoint selection into the existing `telegram-protocol` owner while leaving HTTP/DoH/Firestore request scheduling, HTTP Date clock mutation, temporary-DC scheduling and dc-options application pending.

This round rereads the carrier response boundary before those scheduler concerns:

- `Telegram/SourceFiles/mtproto/details/mtproto_domain_resolver.cpp@eeac8f6a8e2df67967ea7b536fb165e2f8d983e9`
- `Telegram/SourceFiles/mtproto/details/mtproto_domain_resolver.h@cb1979bbbf0454135c8668ed1e5013da85d1dedc`
- `Telegram/SourceFiles/mtproto/special_config_request.cpp@fdb65506b0a886bc2daeccb61090bc44be7e2d88`

The fixed-U code contains a deterministic response codec that is independent of sockets, timers, proxy state and temporary-DC lifecycle:

1. Google and Mozilla/Cloudflare DoH responses use the same JSON `Answer` parser.
2. Address resolution projects answer data in source order and derives a bounded cache TTL.
3. Special-config DoH restricts answers to type 16 (TXT), orders TXT fragments by the source `QMultiMap` length key and concatenates them into the compact payload text.
4. Firestore extracts `fields.data.stringValue` and converts it to Latin-1 bytes.

## 2. Goal

Implement the fixed-U pure response parsing/normalization codec in an existing source-neutral owner, without implementing or pretending to implement carrier scheduling.

## 3. Ownership decision

The logical carrier lifecycle owner remains `native/telegram-network`, but the reviewed desktop mirror contains only the network source mirror and no independent network manifest. The canonical `telegram-network` crate also does not currently depend on a JSON codec.

`native/telegram-protocol` already owns pure wire/payload codecs and already has `serde_json = "1"` in `Cargo.toml@a62d99f23d370cd75590c7b3186307c1689554ed`. Therefore this round places only the deterministic response codec in `telegram-protocol`; a future shipping network scheduler must call it rather than duplicate it.

No second network runtime, settings owner, bootstrap provider or DC directory is introduced.

## 4. Non-goals

- Creating HTTP/DoH/Firestore requests, headers, host camouflage or random padding.
- Racing, retrying, cancelling or timing carrier attempts.
- Updating global time from HTTP `Date`.
- Caching state, wall-clock expiry mutation or callback scheduling.
- Temporary-DC insertion, `help.getConfig`, timeout/cancel/kill lifecycle or `DcOptions::setFromList`.
- Proxy settings/suppression.
- Remote Config / Realtime Database adapters that are present as source helper branches but are not scheduled by the fixed-U constructor reviewed here.
- Executing build, test, lint, validator, generator, dependency resolution, CI, packaging or runtime acceptance in Stage A.

## 5. Requirements

- **R1 — Fixed source identity.** Preserve the semantics of the three fixed-U blobs listed above.
- **R2 — DoH top-level parsing.** Empty input, malformed JSON, non-object root, missing `Answer`, or non-array `Answer` normalizes to an empty result.
- **R3 — DoH entry filtering.** Non-object entries are skipped. With a type restriction, a missing/non-number `type` is skipped and only source-rounded values equal to the restriction are kept. `data` must exist and be a string. Result order follows accepted `Answer` order.
- **R4 — TTL projection.** Missing/non-number TTL normalizes to zero seconds. Address normalization returns no answer for an empty parsed response; otherwise it preserves accepted data and computes the fixed-U effective TTL by starting at 300000 ms and taking the minimum of each entry's TTL multiplied by 1000 and clamped to at least 10000 ms.
- **R5 — TXT normalization.** Special-config DoH applies type 16, sorts TXT fragments by descending source `QString::size()` (UTF-16 code-unit count), and for equal lengths follows `QMultiMap` duplicate ordering (later insertion first), then concatenates and converts to Latin-1 bytes.
- **R6 — Firestore normalization.** Parse an object and project `fields.data.stringValue` when it is a string; malformed/missing/wrong-type structure returns empty bytes. Convert the resulting text with source-equivalent Latin-1 replacement semantics.
- **R7 — Pure boundary.** APIs accept response bytes and typed options and return data only. They must not open sockets, schedule work, read proxy/settings state, mutate a clock, write a cache, change a DC directory or issue MTProto requests.
- **R8 — Existing-owner-first.** Reuse `telegram-protocol` and its existing `serde_json`; do not add a second carrier/runtime or duplicate JSON parser.
- **R9 — Dependency discipline.** No dependency or lock delta is required.
- **R10 — Deferred validation.** Author internal/public contracts but do not execute them in Stage A.
- **R11 — Scheduler split.** Keep actual carrier acquisition, HTTP Date mutation, temporary-DC lifecycle and dc-options application mapped/pending.

## 6. Target API

Add `response_normalization.rs` to the existing protocol crate with:

- `DnsEntry { data, ttl_seconds }`
- `NormalizedDnsResponse { data, ttl_millis }`
- `parse_dns_response(bytes, type_restriction)`
- `normalize_dns_response(bytes, type_restriction)`
- `SpecialConfigResponseKind::{DnsTxt, Firestore}`
- `normalize_special_config_response(kind, bytes)`

The result of `SpecialConfigResponseKind::DnsTxt` or `Firestore` is the compact text bytes consumed by the already implemented special-config payload decoder.

## 7. Failure semantics

Structural DoH/Firestore failures intentionally normalize to empty outputs because fixed-U callers use emptiness as the fail-closed signal. Invalid individual DoH answer entries are skipped without rejecting otherwise valid siblings.

No logging API is added to this codec; secrets/payloads are not logged.

## 8. Verification strategy

Author only; do not run:

- DoH structural failure vectors.
- Mixed Answer entries with type filtering, missing data and TTL defaulting.
- Source-order preservation.
- TTL minimum/maximum clamp vectors.
- TXT descending-length and equal-length duplicate ordering.
- Firestore nested extraction plus malformed/missing fields.
- Latin-1 replacement behavior.
- Public composition contract showing response normalization is independent of network scheduling.

## 9. Acceptance criteria

- **AC-1:** Pure DoH parser and address normalizer are implemented in the existing protocol owner.
- **AC-2:** TXT and Firestore special-config adapters are implemented without I/O.
- **AC-3:** Existing `serde_json` is reused; no manifest/lock change.
- **AC-4:** Internal and public contracts are authored and remain not-run.
- **AC-5:** Canonical platform and desktop source mirrors are byte-identical for the new/changed protocol files.
- **AC-6:** The unique ledger/P0/source-closure/evidence line is updated without creating a second progress truth.
- **AC-7:** Carrier request scheduling/application remains explicitly pending.
- **AC-8:** Stage A makes no executable validation or release claim.

## 10. Provenance

Behavioral provenance only; no upstream C++ is copied.

## 11. Compliance record

Initial spec only. Implementation, authored-contract and bookkeeping evidence will be recorded after the spec-first commit. Executable validation remains intentionally not-run.
