# Telegram Direct HTTP Carrier / Staggered Reply Lifecycle Boundary — Specification

Status: blocked / no-production-delta in this round  
Owner candidate: existing `native/telegram-network`; no parallel Telegram HTTP subsystem authorized  
Last updated: 2026-11-01  
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
