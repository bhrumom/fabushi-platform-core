# Telegram WebProxy canonical address / settings boundary — Specification

Status: active
Owner: existing canonical `native/telegram-protocol` for already-canonical wire/config primitives; unified Fabushi settings owner required for user-input/import/share lifecycle
Last updated: 2026-11-01
Related issue/task/PR: 7457eb1c-11a3-4626-8563-a3e495872536 round 81

## 1. Context / problem

Fixed `telegramdesktop/tdesktop@f985723b552b30fe1c17d377d57f44e61a15a8c4` carries an adjacent WebProxy address/settings responsibility beyond the already-closed bridge-capability/path/link-secret wire slice.

`Telegram/SourceFiles/mtproto/mtproto_proxy_data.cpp@f9748fcb6358dd8ce7429536ec2b999cb2a989ef` defines:
- `NormalizeWebProxyHost`: trim, reject URL delimiters/trailing dot, convert Unicode host input with `QUrl::toAce`, lowercase, enforce DNS label limits, reject numeric-terminal-label IPv4 shorthand and IP literals;
- `StripWebProxyScheme`: strip only case-insensitive `https://`, leaving `http://` to fail;
- `NormalizeWebProxyBasePath` / `ValidWebProxyBasePath`;
- `ProxyData::webAddress` and `ProxyData::setWebAddress`: canonical `host[/base-path]` storage and parsing.

`Telegram/SourceFiles/boxes/connection_box.cpp@e9fa8a1d8f727042e4cadd632182eee5259f80c2` defines the user/settings boundary:
- WebProxy share query `webproxy?server=<url-encoded webAddress>&secret=<path-aware secret>`, with no port;
- local/public link projection through `tg://` / account internal-link domain;
- import through local URL handlers, `server` with legacy `host` fallback, fixed port 443, `setWebAddress`, base64url normalization and path-aware link-secret decode;
- clipboard candidate extraction, settings-list insertion, duplicate handling, user feedback and sharing UI.

The prior responsibility `TDRP-R9-MTPROTO-WEB-PROXY-BRIDGE-CAPABILITY-001` intentionally excludes Unicode/IDNA input normalization and UI/settings sharing integration. The relay carrier/window/bootstrap responsibility remains separately blocked.

## 2. Goal

Determine and record the exact existing-owner-first boundary for fixed-U WebProxy canonical address normalization and config import/share semantics without inventing a carrier scheduler, a second proxy/settings subsystem, or a non-equivalent IDNA implementation.

Where an independently shippable exact responsibility is not currently reachable, keep it `mapped/pending` with explicit unblock conditions rather than adding dead or approximate production code.

## 3. Non-goals / out of scope

- Bridge HMAC/path/link-secret wire already closed by `telegram-web-proxy-bridge-capability-wire.md`.
- Relay frames, logical streams, windows, WebView nonce/bootstrap, reconnect, carrier selection or connection scheduling.
- SOCKS5/HTTP/MTProto proxy settings semantics outside the WebProxy-specific address/link delta.
- QR rendering, clipboard UI, toast/dialog presentation or visual design work.
- Adding a parallel Telegram settings owner.
- Stage-B build, dependency resolution, lock regeneration, lint, test execution, CI/status/check query, packaging, deployment, installation or runtime acceptance.

## 4. Requirements

- R1: Preserve fixed-U source identities and keep canonical-address/settings distinct from bridge wire and relay lifecycle.
- R2: Exact host normalization must match fixed-U `QUrl::toAce` behavior for Unicode input, then lowercase and enforce the fixed-U DNS/IP rejection policy.
- R3: Host input rejects empty input, `:`, `/`, `?`, `#`, `@`, trailing dot, total ASCII result over 253 bytes, no-dot names, empty labels, labels over 63 bytes, leading/trailing `-`, non-ASCII/non-alphanumeric-or-hyphen ACE output, numeric terminal labels, and IP literals.
- R4: Required source vectors include ` Proxy.Example.COM ` -> `proxy.example.com`, `bücher.example` -> `xn--bcher-kva.example`, `bücher.de` -> `xn--bcher-kva.de`, and already-ACE `xn--strae-oqa.example` unchanged.
- R5: Required negative vectors include `localhost`, `127.0.0.1`, `127.1`, `0x7f.1`, `0177.0.0.1`, `1.2.3`, `site.example:443`, `site..example`, and trailing-dot hosts.
- R6: Address input trims whitespace, strips only case-insensitive `https://`, splits at the first slash, canonicalizes the host, validates/normalizes the existing fixed-U base-path grammar, and stores canonical `host[/base-path]`; `http://` remains invalid.
- R7: WebProxy share query path is exactly `webproxy?server=<URL-encoded canonical webAddress>&secret=<EncodeWebProxyLinkSecret>`; no `port` field is emitted.
- R8: WebProxy import uses `server` with legacy `host` fallback, forces port 443, applies the canonical address parser, translates `+` -> `-` and `/` -> `_` in secret text, then applies the existing path-aware link-secret decoder.
- R9: Link routing/settings-list insertion, duplicate handling, clipboard extraction, public-link domain selection, QR and UI feedback remain in the unified settings/application owner, not `telegram-protocol`.
- R10: Existing-owner-first: `telegram-protocol` may own deterministic source-neutral canonicalization/serialization primitives only if exact IDNA behavior is supplied by an already-approved dependency/owner and the primitive has a real settings consumer. Do not create a second settings subsystem.
- R11: Current Stage-A code must not implement an approximate hand-written IDNA/punycode algorithm or silently choose a new IDNA dependency whose equivalence to fixed-U `QUrl::toAce` has not been established.
- R12: If adding a mature IDNA dependency requires dependency resolution / lock regeneration, that executable dependency-resolution step remains deferred under Stage A; do not hand-fabricate Cargo lock data.
- R13: No carrier/scheduler/network production code changes are needed for this responsibility.
- R14: Stage A may author durable spec/contract vectors/bookkeeping only; executable validation remains `not-run`.

## 5. Current state

At canonical checkpoint `bhrumom/fabushi-platform-core checkpoint/tdrp-telegram-auth-lifecycle-7457eb1c-r16@129d232fc891074ec11cd67a4235b78521bcee7c`:
- `native/telegram-protocol/src/web_proxy.rs@4c91f6e4e6b8d37d84afd40aae6af9fbbe7d80d4` validates already-canonical lowercase ASCII hosts and owns the fixed-U base-path/bridge/link-secret primitives.
- `native/telegram-protocol/Cargo.toml@a62d99f23d370cd75590c7b3186307c1689554ed` has no `idna`, `url`, `percent-encoding`, or equivalent Unicode host-normalization dependency.
- `native/telegram-protocol/Cargo.lock@97ad9046ce8926764258c978664cd8cd92fce503` contains no such package.
- The live platform tree contains no unified Fabushi proxy/settings production owner for this WebProxy settings lifecycle. The Telegram conversation adapter is not a settings owner.
- The desktop checkpoint mirrors the protocol/provider sources and migration bookkeeping; it likewise has no canonical proxy/settings production owner to receive this lifecycle.
- `telegram-network` remains unchanged and is not an address/settings owner.

Therefore exact Unicode/IDNA user-input normalization plus import/share settings lifecycle is not currently independently shippable without either inventing a settings owner or introducing an unproven/stale dependency delta.

## 6. Target state

The eventual exact state is:
1. unified settings/application owner accepts user-entered or imported WebProxy address/link;
2. a reviewed exact IDNA/URL primitive normalizes Unicode host input to canonical lowercase ASCII under the fixed-U policy;
3. canonical host/base path/secret are handed to existing `telegram-protocol` bridge/link-secret primitives;
4. the same unified settings owner persists/selects/shares the proxy and routes public/local links;
5. carrier/network lifecycle consumes canonical config later, without owning normalization or settings state.

This round intentionally does not create steps 1-2 because the required owner/dependency is absent.

## 7. Architecture and ownership boundaries

Existing canonical owners:
- `native/telegram-protocol`: deterministic already-canonical WebProxy wire/config primitives.
- unified Fabushi settings/application owner: required future owner for input/import/share/persistence/selection lifecycle.
- `native/telegram-network`: future carrier/socket lifecycle only.

Rejected owners:
- `telegram-network` for Unicode/address/link settings: wrong lifecycle and would mix UI/config with socket scheduling.
- `mahayana-telegram` conversation provider: conversation adapter, not global settings/proxy owner.
- a new Telegram/WebProxy settings subsystem: duplicates unified settings ownership.
- hand-written protocol-local IDNA: high semantic risk and no fixed-U equivalence proof.

## 8. Interfaces / contracts / schemas / data flow

Required future pure boundary:

`raw address -> trim -> strip HTTPS only -> split host/path -> exact IDNA-to-ASCII host normalization -> lowercase/DNS/IP policy -> existing base-path normalizer -> canonical WebProxyAddress`

Share:
`canonical address + secret -> webproxy?server=<encoded host[/path]>&secret=<existing path-aware secret encoding>`

Import:
`webproxy fields -> server|host -> canonical address parser -> secret alphabet normalization -> existing path-aware secret decode -> typed canonical config -> unified settings owner`

No scheduler or socket state participates.

## 9. Constraints and non-functional requirements

- Fail closed on ambiguous/non-canonical host/address/link input.
- Do not log proxy secrets.
- Do not add an unreviewed Unicode/IDNA dependency merely to make this slice appear implemented.
- Do not hand-edit generated dependency lock identities/checksums.
- Keep canonical address deterministic and source-neutral once the IDNA primitive is approved.
- Preserve existing Stage-A zero-CI / zero-executable-validation rules.

## 10. Failure modes and edge cases

- Unicode label maps differently between candidate IDNA implementation and fixed-U Qt behavior.
- Unicode/ACE result exceeds DNS limits.
- numeric-terminal shorthand bypasses ordinary IP parser.
- user supplies `http://`, explicit port, credentials, query, fragment or userinfo in address field.
- base path contains repeated slash, percent escape, dot segment, whitespace, non-ASCII, leading `-`/`_`, or exceeds 128 bytes.
- path-bearing imported link carries an unmarked secret.
- import uses legacy `host` instead of `server`.
- clipboard contains zero or multiple link candidates.
- public-link domain depends on account/session config unavailable to protocol code.

## 11. Implementation strategy

This round is a deliberate no-production-delta disposition:
1. retain current canonical protocol source unchanged;
2. record this exact responsibility as `mapped/pending`;
3. author the fixed-U host/address/import/share vectors in this durable contract;
4. update the unique desktop ledger/P0/source-closure/evidence only;
5. unblock later only when a canonical unified settings owner and reviewed exact IDNA primitive are available without violating Stage-A dependency/lock rules.

## 12. Verification / test strategy

Authored contract vectors, not executed:
- host positive vectors from R4;
- host rejection vectors from R5 plus empty/oversized/invalid-label/delimiter/trailing-dot cases;
- ` HTTPS://Proxy.Example.COM/App/ ` -> canonical `proxy.example.com/App`;
- `http://proxy.example.com` rejects;
- root and multi-segment base-path normalization/rejection vectors from the adjacent bridge spec;
- share query emits `webproxy`, encoded `server=host[/path]`, no port, and existing path-aware secret encoding;
- import accepts `server` and legacy `host`, forces 443, normalizes `+`/`/` secret alphabet, and rejects unmarked path secrets;
- protocol/network production blobs remain unchanged.

Because the exact production API/owner is absent, no executable test referencing a nonexistent API is added solely to manufacture coverage. Stage-B execution remains not-run.

## 13. Acceptance criteria / Definition of Done

- AC-1: fixed-U source identities and exact host/address/import/share semantics are durably recorded.
- AC-2: current owner/dependency inventory proves whether an exact independent implementation exists.
- AC-3: no approximate IDNA, duplicate settings owner, dead codec, scheduler or carrier code is introduced.
- AC-4: unique ledger row is `mapped/pending`, `validation_status=not-run`, `accepted=false`, with explicit unblock conditions.
- AC-5: existing bridge-capability implementation and relay blocker remain unchanged and distinct.
- AC-6: desktop P0/source-closure/evidence record the no-production-delta source review with exact SHAs.
- AC-7: final platform spec compliance records source-review results and the desktop bookkeeping commit.

## 14. Release / migration / rollback

No runtime release, persistence migration or dependency change occurs. Rollback is removal of the spec/bookkeeping evidence only.

## 15. Observability / evidence

Evidence is Git identity:
- fixed-U source blobs;
- platform checkpoint/tree and protocol Cargo/source blobs;
- initial/final spec commits and blobs;
- desktop unique ledger row plus P0/source-closure/evidence commit;
- unchanged canonical/desktop protocol and network production blobs.

## 16. References / provenance

- `telegramdesktop/tdesktop@f985723b552b30fe1c17d377d57f44e61a15a8c4`
- `Telegram/SourceFiles/mtproto/mtproto_proxy_data.cpp@f9748fcb6358dd8ce7429536ec2b999cb2a989ef`
- `Telegram/SourceFiles/mtproto/mtproto_proxy_data.h@d8deebac2359936f72901ba0eec0cbfdfb186e90`
- `Telegram/SourceFiles/boxes/connection_box.cpp@e9fa8a1d8f727042e4cadd632182eee5259f80c2`
- `native/telegram-protocol/src/web_proxy.rs@4c91f6e4e6b8d37d84afd40aae6af9fbbe7d80d4`
- `native/telegram-protocol/Cargo.toml@a62d99f23d370cd75590c7b3186307c1689554ed`
- `native/telegram-protocol/Cargo.lock@97ad9046ce8926764258c978664cd8cd92fce503`
- adjacent bridge spec `docs/specs/telegram-web-proxy-bridge-capability-wire.md@87405d6f79801a891d1837ebdd3a3606a38a6b09`

No upstream C++ is copied into Fabushi.

## 17. Spec compliance record

Initial source disposition. Executable validation intentionally remains not-run.

| Requirement / AC | Status | Evidence / reason |
| --- | --- | --- |
| R1-R9 | passed | Fixed-U source review establishes exact semantics and owner split. |
| R10 | blocked | No live unified Fabushi proxy/settings production owner exists in the two checkpoint trees for this lifecycle. |
| R11 | passed | No approximate IDNA implementation is introduced. |
| R12 | blocked | Protocol crate has no reviewed IDNA/URL dependency; Stage A forbids executing dependency resolution/lock regeneration. |
| R13-R14 | passed | No carrier/network delta and no executable validation. |
| AC-1 | passed | This spec records exact provenance and vectors. |
| AC-2 | passed | Live checkpoint inventory records the missing settings owner and IDNA dependency. |
| AC-3 | passed | Deliberate no-production-delta avoids false ownership/equivalence. |
| AC-4-AC-7 | pending | Await desktop bookkeeping and final source-compliance readback. |
