# Telegram HTTP-derived network time boundary — Specification

Status: active
Owner: existing `native/telegram-network`
Last updated: 2026-11-01
Related issue/task/PR: Fabushi migration task 7457eb1c-11a3-4626-8563-a3e495872536 round 88 / source-evidence round 119

## 1. Context / problem

Fixed upstream remains `telegramdesktop/tdesktop@f985723b552b30fe1c17d377d57f44e61a15a8c4`.

The special-config carrier is still blocked because the canonical native network layer has no first-party direct HTTPS/Web-PKI/timer/cancellable-reply carrier. That blocker does not make the time mutation itself carrier-dependent.

The fixed-U chain separates these responsibilities:

- `special_config_request.cpp@fdb65506b0a886bc2daeccb61090bc44be7e2d88` parses a successful HTTP `Date` header and calls `base::unixtime::http_update(parsed_seconds)`; only then does a time-only request invoke its completion callback.
- `Telegram/lib_base@95a543e426e35938dd632139d0bb828b268c3b4c/base/unixtime.cpp@b39b788a8a125d29d06f637c57c9b52ac24b138e` owns global MTProto-adjusted Unix time and a separate HTTP-derived shift/valid bit. Updating MTProto time clears the HTTP shift and validity. `http_invalidate()` clears validity without rewriting the stored HTTP shift.
- `mtp_instance.cpp@d25fe3b72298fab371854f80186ee8e4777ea087` suppresses duplicate HTTP-time acquisition while HTTP time is valid or a loader is already alive.
- `mtproto_tls_socket.cpp` consumes `base::unixtime::http_now()` when building the TLS camouflage timestamp.
- `config_loader.cpp@5d07404ba07223016c6728b986dcba10628d7194` separately owns special endpoint -> temporary DC -> `help.getConfig` -> `dc_options` application; that lifecycle remains pending.

Fabushi already has an existing shipping time consumer and correction owner in `native/telegram-network/src/lib.rs@3743331b0b1453048a98a21ae4ac2d2b91a2b2a5`: `EstablishedSession` maintains server-time clock correction for message IDs, and the same crate performs real TLS camouflage handshakes. The missing piece is a source-neutral global network-time state that can accept a future parsed HTTP Date without depending on the absent HTTP carrier.

## 2. Goal

Implement the fixed-U global HTTP-derived time mutation as an extension of the existing `telegram-network` owner, independent of HTTP acquisition, and connect the resulting HTTP-adjusted time to the existing shipping TLS camouflage handshake.

## 3. Non-goals / out of scope

- HTTP/HTTPS dispatch, Web PKI, response acquisition, 800 ms racing, reply cancellation/finalization.
- Parsing the HTTP Date string; `telegram-protocol::parse_http_date_unix_seconds` already owns that pure codec.
- Creating or scheduling the time-only `SpecialConfigRequest`; that remains blocked on the HTTP carrier.
- Temporary-DC insertion, `help.getConfig`, timeout/cancel/kill lifecycle, or `DcOptions::setFromList`.
- Replacing the existing per-session MTProto clock offset used for message IDs.
- Persistence of transient clock offsets.
- Executing build, test, lint, validator, generator, dependency resolution, CI, package, deployment, installation, or runtime acceptance in Stage A.

## 4. Requirements

- **R1 — Existing-owner-first.** Add the responsibility to `native/telegram-network`; do not create a new runtime, provider, settings subsystem, or time service crate.
- **R2 — Thread-safe global state.** Maintain a process-global MTProto/base Unix-time shift, HTTP shift, and HTTP-valid bit using thread-safe primitives.
- **R3 — MTProto update semantics.** Match fixed-U non-forced `base::unixtime::update`: ignore a candidate whose shift differs from the current base shift by less than three seconds without locking future updates; after the first material trusted MTProto update, retain that base shift for subsequent non-forced updates. A material accepted update invalidates HTTP calibration by setting HTTP shift to zero and valid=false.
- **R4 — HTTP update semantics.** A parsed HTTP Unix second updates only the HTTP shift relative to the current MTProto/base-adjusted time and marks HTTP time valid. No HTTP object or carrier handle is required.
- **R5 — HTTP now semantics.** `http_now` returns current MTProto/base-adjusted Unix time plus the stored HTTP shift. It does not require valid=true, matching fixed-U `http_now()`.
- **R6 — Invalidate semantics.** Explicit HTTP invalidation clears valid=false without rewriting the stored HTTP shift, matching fixed-U `http_invalidate()`.
- **R7 — Shipping consumer.** The existing TLS camouflage handshake must use HTTP-adjusted network time for the timestamp field. Before any calibration this is equivalent to local Unix time.
- **R8 — Trusted MTProto integration.** Auth-key establishment and authenticated bad-message time correction must update the global MTProto/base network time so they invalidate any prior HTTP calibration. The existing per-session clock offset remains the message-ID owner.
- **R9 — Carrier independence.** Public HTTP-time mutation accepts parsed Unix seconds only; it must not depend on request descriptors, sockets, TLS/Web-PKI HTTP clients, timers, proxy policy, or callbacks.
- **R10 — Dependency discipline.** Use only the Rust standard library and existing crate dependencies; no manifest or lock delta.
- **R11 — Failure semantics.** Wall-clock-before-epoch and checked arithmetic overflow fail closed with a typed network-time error. TLS maps this to the existing invalid-system-clock network failure.
- **R12 — Deferred validation.** Author focused contracts for update/valid/invalidate/MTProto-reset semantics and shipping TLS time injection, but do not execute them in Stage A.
- **R13 — Remaining application boundary.** The blocked carrier and temporary-DC `help.getConfig` / `dc_options` application must remain separately pending and must not be claimed implemented.

## 5. Current state

`telegram-network` reads `SystemTime::now()` directly for MTProto message IDs and the protocol TLS helper reads the wall clock directly for its convenience API. `EstablishedSession` has a per-session `clock_offset_seconds`, but there is no process-global HTTP-derived calibration state and no API for a future HTTP carrier to apply a parsed Date header.

## 6. Target state

Add `native/telegram-network/src/network_time.rs` with:

- `http_time_valid()`;
- `http_time_now()`;
- `update_http_time(http_unix_seconds)`;
- `invalidate_http_time()`;
- an internal trusted `update_mtproto_time(server_unix_seconds)`;
- a typed `NetworkTimeError`.

The module owns only offsets and validity. The HTTP carrier, when implemented later, will call `parse_http_date_unix_seconds` in the protocol owner and pass the parsed seconds to `update_http_time`.

## 7. Architecture and ownership boundaries

`telegram-protocol` remains the pure parser/wire owner. `telegram-network` already owns real socket/session/TLS behavior and is therefore the canonical owner for transient network time. `telegram-runtime` remains the single client lifecycle owner but does not need a parallel clock state.

The existing per-session MTProto offset is retained because it participates in message-ID validation/retry state. The new global state is the source-neutral equivalent of fixed-U `base::unixtime` for network-wide consumers such as TLS camouflage and future special-config expiry/application.

## 8. Interfaces / contracts / schemas / data flow

Future carrier path:

`successful HTTP reply -> protocol Date parser -> update_http_time(parsed_seconds) -> http_time_valid=true -> http_time_now consumer(s)`.

Trusted MTProto path:

`auth handshake / authenticated bad-message time correction -> update_mtproto_time(server_seconds) -> base shift update + HTTP invalidation`.

Current shipping TLS path:

`TcpTransport TLS camouflage handshake -> http_time_now -> prepare_tls_client_hello_with_entropy(timestamp, secure randomness)`.

## 9. Constraints and non-functional requirements

- No new persistence.
- No new dependency or lock input.
- No secret or response-body logging.
- Global state must be safe across native network threads.
- No background task or scheduler is introduced.
- Stage-A executable validation remains forbidden.

## 10. Failure modes and edge cases

- Local wall clock before Unix epoch: fail closed.
- Shift arithmetic overflow: fail closed.
- HTTP invalidation leaves the prior shift stored but marks it invalid; consumers that require freshness must check validity before deciding whether to reacquire.
- MTProto time update resets HTTP shift to zero and invalidates it.
- Multiple successful HTTP Date observations may replace the HTTP shift; fixed-U SpecialConfigRequest can observe more than one successful staggered reply while its owner remains alive.

## 11. Implementation strategy

1. Add the global network-time module using atomics and checked arithmetic.
2. Export only the HTTP-facing functions/error from `telegram-network`; keep trusted MTProto mutation crate-private.
3. Replace the TLS convenience helper call in the existing TCP handshake with the existing explicit-timestamp helper plus the crate's secure-random function.
4. Invoke trusted MTProto time update after successful auth-key establishment and authenticated bad-message time correction.
5. Author module-level contracts against an isolated local state object so tests do not depend on global mutation.
6. Mirror the canonical source changes into the desktop provider copy.
7. Leave HTTP carrier and temporary-DC application pending.

## 12. Verification / test strategy

Author only; do not run:

- default state: base/http now equals injected local wall time and HTTP valid=false;
- HTTP update computes shift relative to MTProto/base-adjusted time and sets valid=true;
- explicit invalidation preserves shift but clears validity;
- sub-three-second MTProto candidate is ignored without locking later correction;
- first material MTProto update resets HTTP shift/validity and later non-forced candidates do not rewrite the global base;
- repeated HTTP updates replace the previous calibration;
- checked-overflow failure;
- source review contract that TLS handshake obtains its timestamp from `http_time_now` and uses injected secure randomness.

## 13. Acceptance criteria / Definition of Done

- **AC-1:** Global HTTP-derived time mutation is implemented in existing `telegram-network` with no HTTP carrier dependency.
- **AC-2:** Real TLS camouflage handshake consumes HTTP-adjusted time.
- **AC-3:** Trusted MTProto time correction invalidates HTTP calibration.
- **AC-4:** No dependency/lock delta and no parallel time/runtime owner.
- **AC-5:** Contracts are authored and remain unexecuted; validation_status=not-run, accepted=false.
- **AC-6:** Carrier and temporary-DC application remain mapped/pending with explicit blockers.
- **AC-7:** Canonical platform source and desktop mirror are byte-identical for changed provider files.
- **AC-8:** Unique desktop ledger/P0/source-closure/evidence is synchronized and remote SHA/blob identities are reread.

## 14. Release / migration / rollback

No release in Stage A. Roll back the isolated network-time/TLS integration commits if source review disproves the fixed-U boundary before Stage B.

## 15. Observability / evidence

Evidence is fixed-U blob identity, canonical/mirror blob identity, authored contract source, and CAS-updated checkpoint commits. There is intentionally no runtime/CI evidence in Stage A.

## 16. References / provenance

- `telegramdesktop/tdesktop@f985723b552b30fe1c17d377d57f44e61a15a8c4`
- `Telegram/SourceFiles/mtproto/special_config_request.cpp@fdb65506b0a886bc2daeccb61090bc44be7e2d88`
- `Telegram/SourceFiles/mtproto/config_loader.cpp@5d07404ba07223016c6728b986dcba10628d7194`
- `Telegram/SourceFiles/mtproto/mtp_instance.cpp@d25fe3b72298fab371854f80186ee8e4777ea087`
- `Telegram/lib_base@95a543e426e35938dd632139d0bb828b268c3b4c/base/unixtime.h@8953d491fe7604df777c18812ba7d8a357c50836`
- `Telegram/lib_base@95a543e426e35938dd632139d0bb828b268c3b4c/base/unixtime.cpp@b39b788a8a125d29d06f637c57c9b52ac24b138e`
- existing response-normalization spec `docs/specs/telegram-doh-special-config-response-normalization-boundary.md@929cfccd65ec2b009a183ccb052c16817a8ca2a1`
- existing carrier blocker spec `docs/specs/telegram-doh-special-config-http-carrier-lifecycle-boundary.md@91c92cadaf5df2db84eacb1111616af942851b67`

Behavioral provenance only; no upstream C++ is copied.

## 17. Spec compliance record

| Requirement / AC | Status | Evidence / reason |
| --- | --- | --- |
| R1-R11 | passed | `native/telegram-network/src/network_time.rs@19b77a6960d6059b7b708ceb4ec13643a1ec0944` implements thread-safe MTProto/base and HTTP shifts plus validity, checked failure semantics, and public carrier-independent mutation APIs. `native/telegram-network/src/lib.rs@e0dc33a728916fb7be916956f44cfb0b670486c6` makes the real TLS camouflage handshake consume `http_time_now` and makes auth/bad-message trusted time correction invalidate HTTP calibration. No manifest or lock changed. |
| R12 | passed | Eight focused module contracts are authored in `network_time.rs@19b77a6960d6059b7b708ceb4ec13643a1ec0944`; none were executed in Stage A. |
| R13 | passed | The direct HTTP carrier remains blocked under `telegram-doh-special-config-http-carrier-lifecycle-boundary.md`; temporary-DC `help.getConfig` and `dc_options` application remain pending and were not fabricated. |
| AC-1-AC-5 | passed | Canonical implementation commit `16e9ea2297b4a8d6d8d41177f74bec927d93f11f`; network-time blob `19b77a6960d6059b7b708ceb4ec13643a1ec0944`; network lib blob `e0dc33a728916fb7be916956f44cfb0b670486c6`. Executable validation remains `not-run`, accepted=false. |
| AC-6 | passed | Carrier plus temporary-DC config application remain separately mapped/pending. |
| AC-7 | passed | Desktop provider source is byte-identical at `source/mahayana/mahayana-rs/providers/telegram-network/src/network_time.rs@19b77a6960d6059b7b708ceb4ec13643a1ec0944` and `src/lib.rs@e0dc33a728916fb7be916956f44cfb0b670486c6`; mirror correction commit `53e1a4b22e94a2d688d47e681ef41e6e4bf4247e`. |
| AC-8 | passed | Desktop bookkeeping commit `45e14b12c765cd8b93ac0e7025be3a0ce84524bd` synchronizes the unique ledger (`8334e9001ecd40d3a760a8cf64dcb4c723b09c35`), P0 (`d27d2a8699bbeed2932233b2f9c6fbaaeff1aa52`), source-closure (`2cf41094a94941d97c5097f4b51ea091b6f06220`) and round119 evidence (`0a5787a9c6386f02c6b8286a0581d8c208d9b2af`) after the byte-identical provider mirror. |
