# Telegram MTProto TLS camouflage wire — Specification

Status: active
Owner: canonical `native/telegram-protocol` wire/cryptography boundary with `native/telegram-network` socket integration
Last updated: 2026-11-01
Related task: 7457eb1c-11a3-4626-8563-a3e495872536 round 78

## 1. Context / problem

Fixed `telegramdesktop/tdesktop@f985723b552b30fe1c17d377d57f44e61a15a8c4` declares the local `tlsClientHello/TlsBlock` family in `Telegram/SourceFiles/mtproto/scheme/mtproto.tl@76225965bfc794388db241f3f760db8ab79e84f8`. Fixed-U `Telegram/SourceFiles/mtproto/details/mtproto_tls_socket.cpp@e7562e3ca1a1da03f5d368085d5e61d887ab3f16` uses that family as an in-memory template AST which emits TLS camouflage bytes; it also validates the camouflage ServerHello and wraps application bytes in TLS-looking records. `Telegram/SourceFiles/mtproto/connection_tcp.cpp@d721a9058e1885d2964ded55bd4e4c7c2ce9e381` selects padded-intermediate MTProto framing for `0xEE` secrets and hands socket lifecycle to the TLS socket without changing request-history semantics.

Round 66 correctly rejected a standalone TL parser/serializer, but treated the whole generator/socket lifecycle as one blocked unit. The live Fabushi boundary is now explicit: `native/telegram-network/src/lib.rs` states that sockets/retry live in telegram-network while wire framing and cryptography live in telegram-protocol. Therefore the TLS camouflage wire layer can be separated from broader socket scheduling and implemented source-neutrally.

## 2. Goal

Implement the exact fixed-U TLS camouflage wire responsibility in the existing telegram-protocol owner and connect it to the existing synchronous TcpTransport path, without introducing a second runtime/provider or changing broader connection scheduling.

## 3. Non-goals / out of scope

- Proxy selection, WebProxySocket behavior, SOCKS/HTTP proxy authentication, or Web transport.
- Connection racing, retry/backoff, reconnect policy, endpoint scheduling, or request-history correlation.
- Auth-key binding/persistence/expiry/renewal.
- gzip or complete difference/differenceSlice decoding/application.
- Treating `tlsClientHello/TlsBlock` as an MTProto request/response object or serializing the TL template itself onto the wire.

## 4. Requirements

- R1: Preserve fixed-U provenance for `tlsClientHello`, all `TlsBlock` variants, `PrepareClientHelloRules`, `Generator::Part::writeBlock`, ServerHello checks, and TLS record framing.
- R2: Keep the TL family as a local template description; the production output is TLS camouflage bytes, not TL wire bytes.
- R3: Existing-owner-first: telegram-protocol owns the reusable wire/cryptographic codec; telegram-network owns TcpStream I/O only.
- R4: Parse `0xEE` secrets as `marker || 16-byte key || domain`, requiring at least 21 bytes exactly as fixed-U socket selection does.
- R5: ClientHello rendering must preserve the fixed template, 2048-byte cap, eight GREASE seeds, random blocks, domain substitution, X25519 public-key blocks, 2-byte big-endian scoped lengths, permutation blocks, M/E blocks, padding rule, first 32-byte zero digest slot, HMAC-SHA256 finalization, and little-endian timestamp XOR in the digest tail.
- R6: HMAC and X25519 must use reviewed general-purpose crates rather than hand-written cryptography: `hmac 0.12.1` (MIT OR Apache-2.0) and `x25519-dalek 2.0.1` (BSD-3-Clause). Existing `getrandom` remains the entropy owner.
- R7: ServerHello parsing must preserve fixed-U record markers, big-endian length boundaries, 65536-byte cap, digest location, client-digest prefix, HMAC verification, and incomplete-input behavior.
- R8: Client application framing must preserve the initial `14 03 03 00 01 01` prefix when the 64-byte obfuscated connection-start prefix is present, `17 03 03` records, 2-byte big-endian lengths, and the 2878-byte client record payload limit.
- R9: Server application deframing must accept only `17 03 03` records, preserve 2-byte big-endian lengths, skip zero-length records, and expose only record payload bytes to the existing obfuscated transport decryptor.
- R10: `TcpTransport::connect_obfuscated` must use this path for `0xEE` secrets before sending the existing obfuscated connection-start prefix; non-`0xEE` transports must remain behaviorally unchanged.
- R11: No broader scheduler/retry/proxy/request-history behavior changes are permitted in this responsibility.
- R12: Stage A authors contracts but runs no build, test, lint, validator, generator, CI, packaging, deployment, installation, or runtime acceptance.

## 5. Current state

At live platform-core checkpoint `b48b9a92c45f7954322946fb7640b53affc5d7bc`:
- telegram-protocol already owns MTProto framing, obfuscated connection-start AES-CTR, secure randomness, and cryptographic wire behavior.
- telegram-network already routes shipping DC candidates through `TcpTransport::connect_obfuscated`.
- `0xEE` secrets are currently reduced to padded-intermediate inner-secret semantics; the domain and TLS camouflage handshake/record layer are not used.
- the prior durable spec/ledger row is mapped/blocked because no shipping TLS camouflage owner existed.

## 6. Target state

`native/telegram-protocol/src/tls.rs` becomes the single source-neutral TLS camouflage wire owner:
- `TlsCamouflageSecret` parses the fixed-U secret shape.
- `prepare_tls_client_hello` and deterministic contract hook render the fixed ClientHello.
- ServerHello inspection validates structure and HMAC without owning socket scheduling.
- client/server TLS-looking record codecs wrap and unwrap arbitrary transport bytes.

`native/telegram-network::TcpTransport` calls those primitives for `0xEE` secrets in the existing synchronous connection path and then continues through the existing obfuscated padded-intermediate codec. Broader retry/racing/reconnect remains unchanged.

## 7. Architecture / ownership

Canonical wire owner: `bhrumom/fabushi-platform-core:native/telegram-protocol`.

Canonical socket/shipping owner: `bhrumom/fabushi-platform-core:native/telegram-network`.

Rejected owners:
- standalone Telegram/TLS provider or second transport runtime;
- telegram-protocol TL encode/decode APIs for the template AST;
- request/session owners, because this responsibility has no arbitrary request-history state.

## 8. Data flow

`0xEE secret -> TlsCamouflageSecret(key, domain) -> fixed template interpreter -> ClientHello bytes + digest -> TcpStream write -> ServerHello bytes -> structural/HMAC verifier -> existing ObfuscatedConnectionStart -> padded-intermediate frame -> AES-CTR -> TLS record wrapper -> TcpStream`.

Inbound reverses the last stages:
`TcpStream -> TLS record deframer -> AES-CTR decrypt -> existing padded-intermediate frame decoder`.

## 9. Failure modes

- invalid or too-short `0xEE` secret;
- entropy unavailable;
- ClientHello exceeds 2048 bytes or a scoped length exceeds u16;
- missing digest slot;
- malformed ServerHello marker/length or length above 65536;
- incomplete ServerHello/record input;
- ServerHello HMAC mismatch;
- invalid application record marker/length;
- client record prefix larger than the 2878-byte payload budget.

All malformed wire cases fail closed with typed errors.

## 10. Verification strategy

Stage A only authors:
- deterministic ClientHello template/digest/length contracts;
- invalid-secret and output-limit contracts;
- synthetic ServerHello HMAC/structure contracts;
- TLS application record framing/deframing contracts;
- local TcpTransport fixture proving `0xEE` shipping performs ClientHello first, validates a synthetic ServerHello, then sends the obfuscated prefix inside TLS records and unwraps inbound records.

No contract is executed in Stage A.

## 11. Acceptance criteria

- AC-1: fixed-U source identities and exact wire semantics are recorded.
- AC-2: telegram-protocol contains the source-neutral TLS camouflage wire owner.
- AC-3: no custom HMAC/X25519 implementation is introduced.
- AC-4: telegram-network shipping uses the wire owner for `0xEE` secrets without changing broader scheduling.
- AC-5: non-`0xEE` transport behavior is preserved.
- AC-6: contracts are authored in canonical owners and mirrored to desktop where that provider surface is mirrored.
- AC-7: unique parity-ledger/P0/source-closure distinguish implemented wire responsibility from still-open proxy/Web/scheduler work.
- AC-8: `validation_status=not-run`, `accepted=false`; no executable evidence is claimed.

## 12. Dependency rationale

`hmac 0.12.1` is the RustCrypto HMAC implementation licensed MIT OR Apache-2.0 and interoperates with the already-used `sha2 0.10`. `x25519-dalek 2.0.1` is BSD-3-Clause, Rust 2021 compatible, and avoids bespoke X25519 arithmetic. Both are narrowly scoped to fixed-U wire generation.

## 13. Rollback

Revert the spec/code/bookkeeping commits together. Non-`0xEE` paths remain isolated, so rollback does not require data migration.

## 14. Stage-A compliance record

This revision supersedes the round66 “whole lifecycle blocked” disposition only for the exact wire/socket-boundary responsibility above. Proxy/Web transport, broader scheduling, arbitrary request history, auth-key lifecycle, gzip, complete difference decoding/application, broader fixed-U source/legal closure, Stage-B executable evidence, and independent release ACCEPT remain open.


## 15. Source-level compliance review — round 109 / task round 78

Status: Stage-A source review complete; executable validation intentionally not run.

- R1/R2: **passed by source review** — fixed-U template/schema and `mtproto_tls_socket.cpp` provenance are retained; the TL family remains local AST and is not exposed as MTProto request/response wire.
- R3: **passed by source review** — canonical `telegram-protocol/src/tls.rs@d15b24a350cc69960efa6f84c509245178917ba9` owns reusable wire/crypto; `telegram-network/src/lib.rs@241cd8ec792070a8de8af9ec6ca5b317e34376d1` owns socket integration.
- R4-R9: **passed by source review / not-run executable validation** — secret parsing, template interpreter, GREASE/random/domain/X25519/scopes/permutation/M/E/padding/HMAC/timestamp, ServerHello structure/HMAC, and application record boundaries are implemented and covered by authored contracts `212728aacd277b38821d110379215f75c2b4b675` and `71f9732b1c81d7bf782ff1eccb2852b1401cfa6f`.
- R6: **passed by dependency/provenance review** — `hmac 0.12.1` (MIT OR Apache-2.0) and `x25519-dalek 2.0.1` (BSD-3-Clause) are declared in `Cargo.toml@a62d99f23d370cd75590c7b3186307c1689554ed`; no bespoke HMAC/X25519 implementation was added.
- R10/R11: **passed by source review / not-run executable validation** — only `0xEE` secret paths enter the TLS handshake/record layer; non-`0xEE` and broader scheduler/request-history logic are unchanged by this responsibility.
- R12: **passed** — no Actions/status/check query or trigger, build, typecheck, lint, validator, generator, unit/contract/integration/E2E execution, package, deployment, installation, or runtime acceptance occurred.

Canonical commits: spec `7313529ae77d1abb99f678e8105ec4525474fe55`, production `e4fb391fc4f9f6e151f35469e0ded7326be39749`, authored-only contracts `9b43bb1bb0bb557a9c1289370f4fdc30a571ad1c`.

Desktop source/test mirror commit `6b914b3089a5d3593c276aa37120f9c2f53db198` is byte-identical for TLS codec, protocol lib, network lib and both authored contract blobs. Desktop bookkeeping/evidence commit `ed07df89a6e00800c5ebb436dc5d8e39f8c440a5` records the unique responsibility as `implemented / rewritten / validation_status=not-run / accepted=false`; ledger blob is `49fce8394f92824dfe756945d8c135cb594eb20c` and coverage remains `source_entries_total=16202, responsibilities_total=109, unknown=16172, unread=16145, omitted=0, unmapped_responsibilities=0`.

Adjacent fixed-U `WebProxySocket` remains explicitly outside this closure: it owns `WebProxy::Transport` stream registration, flow-control window grants, buffering and connected/disconnected/failed lifecycle. Broader proxy selection/Web transport, connection racing/reconnect/send retry, arbitrary request history, auth-key binding/persistence, gzip, complete difference decoding/application, remaining source/legal closure, Stage-B executable evidence and independent release ACCEPT remain open.
