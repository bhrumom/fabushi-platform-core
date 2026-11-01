# Telegram Direct HTTP Carrier / Staggered Reply Lifecycle Boundary — Specification

Status: implementation authorized / validation not run  
Owner candidate: existing `native/telegram-network`; no parallel Telegram HTTP subsystem authorized  
Last updated: 2026-11-01 (round 94)  
Related task: Fabushi migration task 7457eb1c-11a3-4626-8563-a3e495872536 round 87 / source-evidence round 118

## 1. Context and fixed source

Fixed upstream remains `telegramdesktop/tdesktop@f985723b552b30fe1c17d377d57f44e61a15a8c4`.

This review fixes the direct HTTP / racing / reply-lifecycle boundary to these source blobs:

- `Telegram/SourceFiles/mtproto/details/mtproto_domain_resolver.cpp@eeac8f6a8e2df67967ea7b536fb165e2f8d983e9`
- `Telegram/SourceFiles/mtproto/details/mtproto_domain_resolver.h@cb1979bbbf0454135c8668ed1e5013da85d1dedc`
- `Telegram/SourceFiles/mtproto/special_config_request.cpp@fdb65506b0a886bc2daeccb61090bc44be7e2d88`
- `Telegram/SourceFiles/mtproto/special_config_request.h@91901489e778089b03187682a9556e929867b94f`

Pure request/response normalization is already isolated in existing Fabushi owners. This spec concerns only the remaining carrier and reply lifecycle.

## 2. Fixed-U carrier semantics

### 2.1 Direct no-proxy dispatch

Both `DomainResolver` and `SpecialConfigRequest` construct a dedicated `QNetworkAccessManager` and set `QNetworkProxy::NoProxy` before any dispatch. This is an explicit carrier invariant: these service requests must not inherit user, system, environment, PAC/WPAD, or Telegram-selected proxy routing.

`DomainResolver::performRequest` dispatches GET requests only. Each reply is tracked under its `AttemptKey`.

The currently seeded `SpecialConfigRequest` attempts are Google, Mozilla, and (outside time-only/test restrictions) Firestore; all have empty payloads and therefore use GET. The source still contains RemoteConfig/Realtime branches and a POST path for a non-empty payload, but those attempt types are not seeded by this fixed-U constructor and are not treated as shipping reachability here.

### 2.2 800 ms staggered scheduling

Both classes use `kSendNextTimeout = 800 ms`.

For `DomainResolver`, scheduling is per `AttemptKey`. When another attempt remains, `sendNextRequest` schedules the next dispatch against the per-key `Attempts.guard`. A later successful DNS result erases that `_attempts[key]` entry, destroying the guard and preventing any not-yet-fired delayed callback for that key.

For `SpecialConfigRequest`, the delayed callback is bound to `this`, not to a per-attempt winner guard. A successful response does not internally clear `_attempts` or stop the remaining staggered attempts. Remaining attempts continue while the object remains alive.

These are intentionally different lifecycle contracts and must not be collapsed into one generic "first reply wins and cancels everything" policy.

### 2.3 ServiceWebRequest finalization and cancellation boundary

`ServiceWebRequest` is a move-only wrapper around a `QPointer<QNetworkReply>`.

- Move construction transfers the pointer.
- Move assignment calls `destroy()` on the currently held reply when replacing it, then transfers the new pointer.
- `destroy()` disconnects the reply's `finished` signal, calls `abort()`, and schedules `deleteLater()`.
- The ordinary destructor does **not** call `destroy()`; if a reply remains, it only schedules `deleteLater()`.

Therefore an ordinary vector/map erase is not source-equivalent to an explicit `abort()` call. A Fabushi carrier must preserve externally visible settlement/cancellation semantics without falsely claiming that every discarded source reply is explicitly aborted.

For each finished reply, both owners read the full response body and remove that reply wrapper from the tracking collection even when the reply reports an error. Error status does not by itself discard a readable body.

### 2.4 DomainResolver settlement

A finished DomainResolver reply is parsed after finalization.

- Empty/invalid normalized DNS response: the race stays live; other running and scheduled attempts may still produce a result.
- Non-empty DNS response: the resolver erases both `_requests[key]` and `_attempts[key]`, computes the bounded TTL cache entry, then publishes the combined result when eligible.
- Erasing `_attempts[key]` is the source mechanism that fences future delayed attempts for the settled key.
- Outstanding reply wrappers are dropped through their ordinary destructor path; source code does not call `ServiceWebRequest::destroy()` in this winner path.

### 2.5 SpecialConfigRequest settlement

A finished special-config reply first exposes its successful HTTP `Date` header to the separate global-time application responsibility, then reads/removes the reply.

If the request is config-bearing and the body is non-empty, the response adapter/decrypt/config application path is invoked. This class does not internally stop the remaining attempt schedule after one decodable response. Carrier code must not add DomainResolver-style first-success cancellation unless the higher-level object lifetime requires it.

Global HTTP-time mutation, simple-config decryption/application, temporary-DC `help.getConfig`, and `dc_options` replacement/change notification are outside this carrier spec.

## 3. Existing-owner-first audit

The canonical current `native/telegram-network` owner is a synchronous MTProto TCP transport:

- `native/telegram-network/Cargo.toml@9ab75b9fa7d8a97f49ef4d627008727826337162` depends on `fabushi-telegram-protocol`, `getrandom`, `thiserror`, and `zeroize` only.
- `native/telegram-network/src/lib.rs@3743331b0b1453048a98a21ae4ac2d2b91a2b2a5` owns `std::net::TcpStream`, MTProto framing/handshake/retry, and the pure request-normalization module. It has no HTTP request, TLS Web PKI, timer, task runtime, or cancellable HTTP reply owner.
- The crate lock contains no `reqwest`, `hyper`, `ureq`, `rustls`, or `native-tls` HTTP/TLS stack.
- `native/telegram-runtime/Cargo.toml@a0d5aee63c886b11c3ad8fbf37885e0a3c5b6053` likewise has no HTTP client/timer runtime dependency and remains the synchronous C/JSON application runtime above `telegram-network`.

The repository also contains vendored `third_party/mahayana/codex-rs/http-client`. Its own README says it is the low-level HTTP transport shared by Codex crates, and its Cargo manifest depends on the Codex workspace's reqwest/rustls/tokio stack. It is not the canonical first-party Telegram/native network owner. Wiring Telegram special-config bootstrap to it would invert ownership into a vendored agent stack and would import proxy-routing/logging policy that is not source-equivalent to the fixed-U dedicated `NoProxy` manager. It is therefore not reused as the Telegram carrier.

No reusable first-party native HTTP carrier/lifecycle owner is present at the reviewed heads.

## 4. Blocker and no-production-delta decision

Actual implementation requires one shipping carrier capable of all of the following together:

1. HTTPS with normal Web PKI validation.
2. Explicit direct/no-proxy mode that cannot silently inherit ambient proxy policy.
3. Applying the existing normalized method/URL/query/Host/Accept/User-Agent descriptors.
4. Returning response status/error, headers, and full body.
5. A cancellable in-flight handle with deterministic drop/finalization semantics.
6. An 800 ms staggered scheduler whose cancellation scope can differ between DomainResolver and SpecialConfigRequest.
7. Integration into the real `telegram-network` / runtime shipping path.

The current owner has none of the required HTTP/TLS/task primitives. Adding a new HTTP/TLS/task dependency graph would require a deliberate shared carrier design and dependency-lock update; Stage A explicitly forbids dependency resolution and lock regeneration. A fake trait-only facade, unconnected race state machine, thread-sleep surrogate, system `curl` subprocess, or tests against a nonexistent carrier would not satisfy production reachability and would create a second progress truth.

Therefore this round is intentionally **blocked / no-production-delta** for the carrier. Existing request normalization production code remains unchanged.

Unblock condition: establish or approve a first-party reusable direct HTTP carrier in the canonical native networking layer, with its dependency/lock inputs available for a later Stage-A source change without executing validation. Then implement this spec inside the existing `telegram-network` owner and wire it into the shipping runtime.

## 5. Authored contract requirements

No executable contract file is added against a nonexistent carrier. When the carrier exists, authored-only Stage-A contracts must cover:

- direct/no-proxy enforcement;
- exact 800 ms dispatch spacing using an injected/fake clock, without sleeping in tests;
- DomainResolver: invalid/empty reply does not settle; first non-empty parsed DNS result fences future delayed attempts; remaining in-flight handles are finalized without inventing stronger abort guarantees than the carrier exposes;
- SpecialConfigRequest: one successful response does not automatically cancel remaining scheduled attempts while the owner stays alive;
- reply body remains observable even with transport error metadata;
- per-reply finalization removes exactly that request handle;
- owner destruction cancels/finalizes all still-owned handles;
- HTTP Date exposure stays separate from global clock mutation;
- no request payload/secret/body logging.

All executable validation remains not-run until Stage B.

## 6. Acceptance criteria for this review

- **AC-1:** Fixed-U .cpp/.h carrier and `ServiceWebRequest` semantics are reread and pinned by blob SHA.
- **AC-2:** Existing `native/telegram-network` and `native/telegram-runtime` are checked for a reusable HTTP/TLS/timer/cancellation owner.
- **AC-3:** Vendored Codex HTTP transport is explicitly rejected as the Telegram carrier owner.
- **AC-4:** DomainResolver and SpecialConfigRequest stagger/winner semantics remain distinct.
- **AC-5:** No fake/unreachable production abstraction or executable test is added.
- **AC-6:** Carrier/application remains `mapped / pending / validation_status=not-run / accepted=false` with a concrete unblock condition.
- **AC-7:** No CI/status/check query or executable validation is performed.

## 7. Provenance

Behavioral provenance only; no upstream C++ is copied.


## 8. Round 94 implementation authorization

The round-87 blocker record above remains valid for the then-current `telegram-network` dependency set: no reusable first-party HTTP carrier existed. Round 94 adds a new existing-owner-first implementation **inside the same `native/telegram-network` owner** rather than borrowing another product crate.

A repository-local dependency audit found an exact Web-PKI closure already pinned in `native/global-dharma/Cargo.lock`: `rustls 0.23.41`, `rustls-pki-types 1.15.0`, `rustls-webpki 0.103.13`, `ring 0.17.14`, `webpki-roots 0.26.11/1.0.8` and their platform/build dependencies. This lock is dependency-input evidence only; `global-dharma` code and lifecycle remain rejected as Telegram owners.

Stage-A implementation SHALL:
1. add a source-neutral direct HTTPS module under `native/telegram-network`;
2. use `TcpStream` + `rustls` + `webpki-roots` for HTTPS/Web-PKI, with TLS SNI bound to the normalized URL host;
3. never consult environment/system/PAC/Telegram proxy settings and never route through a proxy;
4. support the fixed-U seeded GET requests only; RemoteConfig/Realtime POST reachability remains outside this slice;
5. preserve normalized `Host`, `Accept`, and `User-Agent` headers while treating non-2xx HTTP status as response metadata so readable bodies remain observable;
6. reuse the existing `NetworkAttemptControl` so explicit destruction can shut down the active socket, while ordinary reply finalization does not invent an explicit abort;
7. own request handles in a first-party request owner whose drop explicitly cancels still-owned in-flight handles;
8. provide an 800 ms stagger plan with distinct settlement policies: DomainResolver-style acceptance fences future delayed attempts and releases already-running handles without explicit abort; SpecialConfig-style acceptance does not fence the remaining schedule while the owner remains alive;
9. bound response headers/body and reject malformed/chunk framing fail-closed;
10. author focused contracts for no-proxy request construction, exact 800 ms scheduling, distinct settlement/finalization semantics, readable non-2xx bodies, explicit cancellation, and owner-drop cancellation, but execute none.

No dependency resolver or lock generator may be executed. The exact required package records may be transcribed from the already-pinned `native/global-dharma/Cargo.lock` into `native/telegram-network/Cargo.lock`, and the root package dependency list may be edited by hand. This is a source edit, not executable validation.

This round does not yet claim the higher-level RuntimeClient endpoint-acquisition/application composition is complete merely because the carrier exists. Until a real caller composes the carrier with the existing request/response normalization and special-config application chain, the aggregate parity item remains pending / validation_status=not-run / accepted=false.

### Round 94 source-level acceptance criteria

- R94-AC-1: exact fixed-U NoProxy, 800 ms stagger, ServiceWebRequest finalization/destruction, and DomainResolver-vs-SpecialConfig settlement semantics remain pinned.
- R94-AC-2: concrete direct HTTPS/Web-PKI transport lives in the existing `telegram-network` owner; no second provider/runtime/network product is added.
- R94-AC-3: direct mode cannot inherit ambient proxy configuration.
- R94-AC-4: request ownership supports explicit cancellation and ordinary non-aborting finalization as distinct operations.
- R94-AC-5: DomainResolver-style and SpecialConfig-style stagger settlement remain distinct.
- R94-AC-6: exact dependency/lock inputs are source-pinned without executing dependency resolution or lock regeneration.
- R94-AC-7: focused contracts are authored but not run.
- R94-AC-8: aggregate carrier/application parity stays pending until runtime composition is production-wired and Stage-B evidence exists.
- R94-AC-9: no Actions/status/check query or executable validation is performed.


## 9. Round 94 source-compliance record

| Requirement / AC | Source-level status | Evidence |
| --- | --- | --- |
| R94-AC-1 fixed-U carrier lifecycle | passed for source review | `special_config_request.cpp@fdb65506b0a886bc2daeccb61090bc44be7e2d88`, `special_config_request.h@91901489e778089b03187682a9556e929867b94f`, `mtproto_domain_resolver.cpp@eeac8f6a8e2df67967ea7b536fb165e2f8d983e9`, `mtproto_domain_resolver.h@cb1979bbbf0454135c8668ed1e5013da85d1dedc` were reread at fixed U. |
| R94-AC-2 direct HTTPS/Web-PKI owner | implemented, validation not run | platform commit `dc2541aa769fb8fd3f9699683ba802a6a7afddbe`; `direct_https.rs@0c7444d23fdee0aa135f12fd55a50ec3fe42c690`; `lib.rs@bace7702a74f1c3b848ac12add69d8cd74748342`. |
| R94-AC-3 no-proxy boundary | implemented, validation not run | direct carrier opens `TcpStream` to normalized origin host:443 and has no proxy/environment/PAC/WPAD integration; request contracts assert origin-form serialization and no Proxy-* header. |
| R94-AC-4 cancellation/finalization split | implemented, validation not run | `DirectHttpsRace::finalize_reply` releases one handle without cancellation; `cancel_reply` invokes `NetworkAttemptControl::cancel`; Drop explicitly cancels still-owned handles. |
| R94-AC-5 distinct stagger settlement | implemented, validation not run | `SERVICE_REQUEST_STAGGER=800ms`; DomainResolver policy fences future schedule and releases remaining handles without explicit abort, while SpecialConfig policy leaves remaining schedule owned/live. |
| R94-AC-6 dependency inputs | source-pinned, validation not run | `Cargo.toml@51bd7e2a4ac50b76c3bad11e9aef7ac633f6cfef`; manually transcribed `Cargo.lock@d0d7968508c43b433f27cf0e8f2fc7aea33a4953`; source dependency closure came from already-pinned `native/global-dharma/Cargo.lock`. No dependency resolution or lock generation executed. |
| R94-AC-7 contracts | authored, not run | pure tests inside `direct_https.rs` cover request shaping, exact 800ms constant, readable non-2xx body, chunked decoding, settlement split, and explicit cancellation vs finalization. |
| R94-AC-8 aggregate responsibility | pending | carrier is now independently implemented, but no new HTTP-acquisition-to-RuntimeClient orchestration was introduced in this slice; the existing resolved-endpoint -> temporary help.getConfig -> dc_options path remains the canonical downstream composition. |
| R94-AC-9 zero CI | passed for this source-edit round | no Actions/status/check query, build, typecheck, lint, validator, generator, dependency resolution, lock regeneration, test execution, packaging, deployment, installation, or runtime acceptance was performed. |

Dependency provenance was recorded at platform commit `cbe7e60632397e705725db7b30c41ce4bef80eb7` in `native/telegram-network/THIRD_PARTY_NOTICES.md@2e84b5a0ad5d80be35939ee488e4c56b07f9125c`.
The desktop source mirror commit is `623936831ac1831022a35b22471a58bd96036104`; mirrored carrier/lib/notices blobs are byte-identical to platform-core.


### Round 94 lock metadata correction

A post-write source-only lock review found that the source lock used by the transcribed `ring 0.17.14` record contained only `getrandom 0.2.17`, whereas the target telegram-network lock already contained both `getrandom 0.2.17` and `0.3.4`. The target record was therefore corrected from the source's unqualified `"getrandom"` spelling to `"getrandom 0.2.17"` so the hand-authored lock remains unambiguous without invoking Cargo. Correction commit: `3b9368d2f3856d320557945226d8c8b039e2aed8`; corrected lock blob: `d0d7968508c43b433f27cf0e8f2fc7aea33a4953`. A source-text audit of the newly added package records found no remaining unqualified dependency whose package name has multiple versions in the target lock. No dependency resolver, generator, build, or test was run.
