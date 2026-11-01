# Telegram WebProxy bridge capability/link-secret boundary — Specification

Status: active
Owner: canonical `native/telegram-protocol` WebProxy config/wire boundary
Last updated: 2026-11-01
Related issue/task/PR: 7457eb1c-11a3-4626-8563-a3e495872536 round 80

## 1. Context / problem

Fixed `telegramdesktop/tdesktop@f985723b552b30fe1c17d377d57f44e61a15a8c4` separates a pure WebProxy bridge configuration boundary from the carrier lifecycle. `Telegram/SourceFiles/mtproto/mtproto_proxy_data.cpp@f9748fcb6358dd8ce7429536ec2b999cb2a989ef` computes a domain-separated HMAC capability, canonical bridge path/URL, and path-aware link-secret marker encoding. `web_proxy_webview.cpp@8a2efc8fb15e400366f199cd956effc8ad362da8` and `web_proxy_transport.cpp@e25ad970e84686e9aca061d02c274048793b5ad4` consume the resulting URL/path, while `boxes/connection_box.cpp@e9fa8a1d8f727042e4cadd632182eee5259f80c2` consumes the link-secret codec.

The prior relay responsibility `TDRP-R9-MTPROTO-WEB-PROXY-RELAY-STREAM-001` correctly remains blocked on a real shipping carrier. This spec asks whether the capability/path/link-secret responsibility can be closed independently without inventing a carrier, scheduler, second runtime, or dead relay codec.

## 2. Goal

Implement the exact fixed-U WebProxy bridge capability/path/link-secret config/wire boundary in the existing canonical `telegram-protocol` owner, with authored-only Stage-A contracts and byte-identical desktop provider mirrors.

## 3. Non-goals / out of scope

- WebProxy relay frame parsing, stream multiplexing, send/receive windows, WebView nonce/bootstrap, browser adoption, health, retry, reconnect, or carrier scheduling.
- Full user-facing proxy address parsing, Unicode/IDNA conversion, clipboard/link routing, QR/UI, settings persistence, or proxy selection.
- SOCKS/HTTP proxy behavior.
- Connection racing, arbitrary request history, auth-key binding/persistence, gzip, or complete difference decoding/application.
- Any Stage-B build/test/CI/package/deploy/install/runtime execution.

## 4. Requirements

- R1: Preserve the exact fixed-U source identities and keep this responsibility separate from the blocked relay carrier/window lifecycle.
- R2: Existing-owner-first: reusable WebProxy config/wire primitives belong to canonical `native/telegram-protocol`; no new provider/runtime/scheduler is introduced.
- R3: Capability input is a canonical WebProxy host plus canonical base path and raw MTProto proxy secret. The wire owner validates canonical ASCII host/path form but does not perform Unicode/IDNA user-input normalization.
- R4: Root capability context is byte-for-byte `tdesktop-web-proxy-bridge-v1\n<host>`.
- R5: Non-root capability context is byte-for-byte `tdesktop-web-proxy-bridge-v2\n<host>\n<base-path>`, preserving base-path case.
- R6: Capability is HMAC-SHA256 keyed by the decoded proxy secret and encoded as unpadded URL-safe base64. HMAC must use the already-reviewed `hmac 0.12.1` + `sha2 0.10` dependencies; no bespoke HMAC is permitted.
- R7: Canonical base path is root-empty or 1..128 ASCII bytes; one optional leading/trailing slash is trimmed; segments are slash-separated and nonempty; each segment starts with ASCII alphanumeric and continues with ASCII alphanumeric, `-`, or `_`; `.`, `..`, percent escapes, spaces, repeated slashes, and leading `-`/`_` are rejected.
- R8: Bridge path is `/` for root and `/<base-path>/` otherwise. Bridge URL is HTTPS on the canonical host with that exact path and one `bridge=<capability>` query item.
- R9: WebProxy secret text accepts the successful fixed-U Web-proxy shapes: 16 raw bytes or 17 raw bytes beginning `0xDD`, supplied as even hexadecimal or URL-safe base64 text. `0xEE`/21+ TLS-camouflage secrets remain unsupported for WebProxy exactly as fixed-U `ProxyData::status` does.
- R10: A non-root shared link encodes `0x70 || raw-secret` as unpadded URL-safe base64. Root links retain the caller's original secret text.
- R11: Link decode recognizes a decoded value of at least 17 bytes whose first byte is `0x70` and returns the remaining bytes as lowercase hex. An unmarked link secret is accepted only for root; when a base path is present it fails closed.
- R12: The `0x70` marker must remain distinct from legacy `0xDD` 17-byte padded-intermediate secrets so old clients reject path-bearing links instead of silently treating them as pathless.
- R13: No WebView nonce, fragment, navigation/origin check, relay Hello/Welcome, stream/window state, or socket scheduling is added by this responsibility.
- R14: Stage A authors deterministic contracts but runs no build, test, lint, validator, generator, CI/status/check query, package, deploy, install, or runtime acceptance.

## 5. Current state

At platform checkpoint `0fe86b6d40d1e7ad05b7f4a72ce2c8945cf06a6a`:
- `native/telegram-protocol` already owns MTProto transport-secret semantics and directly depends on `hmac`, `sha2`, and `hex`.
- `native/telegram-network` owns sockets and broader connection lifecycle.
- The live canonical and desktop network owner contains no selectable WebProxy carrier; relay/window responsibility therefore remains `mapped/pending`.
- No canonical Rust symbol currently carries the pure bridge capability/path/link-secret boundary.

## 6. Target state

`native/telegram-protocol/src/web_proxy.rs` becomes the pure source-neutral owner for:
- canonical base-path normalization/validation;
- canonical host validation at the wire boundary;
- successful WebProxy secret-text decoding;
- v1/v2 domain-separated bridge HMAC;
- bridge path and URL construction;
- path-aware link-secret encode/decode.

`native/telegram-protocol/src/lib.rs` exports those primitives. No network/carrier production code changes are required because this responsibility is explicitly the independently consumable config/wire contract, not the carrier.

## 7. Architecture and ownership boundaries

Canonical owner: `bhrumom/fabushi-platform-core:native/telegram-protocol`.

Rejected owners:
- `telegram-network` for HMAC/link grammar: it owns sockets/scheduling and currently has no WebProxy carrier; putting pure secret/config wire there would duplicate protocol-level secret semantics and add crypto dependencies unnecessarily.
- standalone WebProxy/Telegram provider/runtime: duplicates canonical ownership.
- UI/settings owner: address input, IDNA normalization, sharing surfaces and persistence are adjacent but not this wire contract.

## 8. Interfaces / contracts / schemas / data flow

`canonical host + base-path input -> normalize/validate path -> parse WebProxy secret text -> raw secret`

Then:
- capability: `context(host,path) -> HMAC-SHA256(raw-secret, context) -> base64url-no-pad`;
- bridge URL: `https://host + bridge-path + ?bridge=capability`;
- shared link secret: root returns original text; non-root returns `base64url-no-pad(0x70 || raw-secret)`;
- imported link secret: marked form returns lowercase hex; unmarked form is permitted only for root.

## 9. Constraints and non-functional requirements

- No new dependency is required.
- No secret value is logged or persisted by this module.
- Pure functions are deterministic and side-effect free.
- Fail closed on non-canonical host/path or malformed/unsupported secret text.
- Keep relay/carrier lifecycle blocked independently.

## 10. Failure modes and edge cases

- uppercase/non-canonical/non-ASCII host reaches the wire owner;
- host lacks a dot, has empty/oversized labels, forbidden delimiters, trailing dot, or a numeric terminal label;
- invalid/oversized/ambiguous base path;
- malformed hex/base64url secret;
- decoded secret shape not 16 bytes or 17 bytes starting `0xDD`;
- `0xEE` WebProxy secret;
- unmarked path-bearing link secret;
- invalid base64url quantum/padding.

## 11. Implementation strategy

1. Add the pure module and typed error.
2. Export it from the existing protocol crate.
3. Add deterministic source-level contract tests to the existing `protocol_contract.rs`; do not execute them.
4. Mirror canonical protocol source/test bytes into desktop provider paths.
5. Add one unique ledger responsibility and synchronize P0/source-closure/evidence.
6. Perform source-level spec compliance review only.

## 12. Verification / test strategy

Authored-only Stage-A contracts cover:
- four fixed-U capability vectors: root/path × 16-byte/plain and 17-byte/`0xDD` secrets;
- v1/v2 domain separation;
- bridge path and URL output;
- base-path positive/negative grammar vectors;
- path link-secret vector `cIVhlEBk_HMMv6RHNWLY7Fk`;
- marked decode and unmarked root/path behavior;
- malformed base64url and unsupported `0xEE` failures;
- no changes to network/carrier production blobs.

No contract is executed in Stage A.

## 13. Acceptance criteria / Definition of Done

- AC-1: fixed-U source identities and exact pure boundary are durably recorded.
- AC-2: canonical telegram-protocol contains the pure implementation and exports it.
- AC-3: HMAC uses existing reviewed dependencies; no new dependency or bespoke cryptography is introduced.
- AC-4: authored contracts contain exact fixed-U vectors and are not executed.
- AC-5: desktop mirrors are byte-identical to canonical source/test blobs.
- AC-6: unique ledger/P0/source-closure distinguish this implemented pure boundary from the still-blocked relay carrier/window lifecycle.
- AC-7: `validation_status=not-run`, `accepted=false`, and no executable evidence is claimed.

## 14. Release / migration / rollback

No runtime release or data migration occurs in Stage A. Rollback is a source/spec/bookkeeping revert; no persistent state migration exists.

## 15. Observability / evidence

Evidence is Git object identity: platform spec/source/test commits and blobs, byte-identical desktop mirrors, unique ledger row, P0/source-closure/evidence commit, and final source-compliance spec revision.

## 16. References / provenance

- `telegramdesktop/tdesktop@f985723b552b30fe1c17d377d57f44e61a15a8c4`
- `Telegram/SourceFiles/mtproto/mtproto_proxy_data.cpp@f9748fcb6358dd8ce7429536ec2b999cb2a989ef`
- `Telegram/SourceFiles/mtproto/mtproto_proxy_data.h@d8deebac2359936f72901ba0eec0cbfdfb186e90`
- `Telegram/SourceFiles/mtproto/web_proxy/web_proxy_webview.cpp@8a2efc8fb15e400366f199cd956effc8ad362da8`
- `Telegram/SourceFiles/mtproto/web_proxy/web_proxy_transport.cpp@e25ad970e84686e9aca061d02c274048793b5ad4`
- `Telegram/SourceFiles/boxes/connection_box.cpp@e9fa8a1d8f727042e4cadd632182eee5259f80c2`
- Adjacent blocked relay spec: `docs/specs/telegram-web-proxy-relay-stream-boundary.md@4c8faf410867a0784623b9e00e73d7a9f46bc3cf`

No upstream C++ is copied into Fabushi.

## 17. Spec compliance record

| Requirement / AC | Status | Evidence / reason |
| --- | --- | --- |
| R1-R14 | pending | Implementation/source review not yet committed. |
| AC-1 | passed | This spec records exact fixed-U provenance and split boundary. |
| AC-2-AC-6 | pending | Await implementation/mirror/bookkeeping. |
| AC-7 | passed | Stage-A executable validation remains prohibited/not-run. |
