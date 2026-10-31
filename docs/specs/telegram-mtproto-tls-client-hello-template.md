# Telegram MTProto TLS ClientHello Template Disposition — Specification

Status: active  
Owner: canonical Telegram protocol/network boundary  
Last updated: 2026-11-01  
Related task: 7457eb1c-11a3-4626-8563-a3e495872536 round 66

## 1. Context / problem

Fixed `telegramdesktop/tdesktop@f985723b552b30fe1c17d377d57f44e61a15a8c4` declares, immediately after `help.configSimple`, `tlsClientHello blocks:vector<TlsBlock> = TlsClientHello` plus the `tlsBlockString`, `tlsBlockRandom`, `tlsBlockZero`, `tlsBlockDomain`, `tlsBlockGrease`, `tlsBlockPublicKey`, `tlsBlockScope`, `tlsBlockPermutation`, `tlsBlockM`, `tlsBlockE`, and `tlsBlockPadding` family in `Telegram/SourceFiles/mtproto/scheme/mtproto.tl@76225965bfc794388db241f3f760db8ab79e84f8`.

Fixed-U `Telegram/SourceFiles/mtproto/details/mtproto_tls_socket.cpp@e7562e3ca1a1da03f5d368085d5e61d887ab3f16` uses these generated TL types as a local template AST. `PrepareClientHelloRules()` constructs the tree and `Generator::Part::writeBlock` interprets it to emit TLS camouflage ClientHello bytes. The family is not serialized as an MTProto request/response object on the network.

## 2. Goal

Record the exact ownership/disposition of this adjacent family without inventing dead protocol APIs or conflating local TLS-template semantics with connection/TLS scheduling.

## 3. Non-goals / out of scope

- TLS socket establishment, retries, routing, or connection scheduling.
- TLS ClientHello byte generation.
- randomness, grease selection, X25519 key generation, HMAC/timestamp injection, padding policy, permutation choice, or scoped-length application.
- auth-key binding/persistence, arbitrary request history, gzip, or complete updates difference decoding/application.
- introducing a standalone TL parser/serializer for objects that have no shipping wire entrypoint.

## 4. Requirements

- R1: Preserve fixed-U provenance for the complete `tlsClientHello/TlsBlock` family.
- R2: Classify the family as a local typed template AST consumed by the TLS camouflage generator, not an inbound/outbound MTProto wire object.
- R3: Existing-owner-first: prefer the canonical telegram-network transport/TLS owner for the eventual generator lifecycle; do not create a second Telegram runtime/provider.
- R4: Do not add telegram-protocol encode/decode APIs unless a real shipping consumer exists.
- R5: Keep all TLS scheduling and cryptographic/generation behavior outside any future schema-only type representation.
- R6: Stage A performs no executable verification.

## 5. Current state

The canonical `native/telegram-protocol` owner has no `TlsClientHello` or `TlsBlock` API. The canonical `native/telegram-network` shipping transport is direct TCP and has no equivalent TLS camouflage ClientHello generator lifecycle. Desktop mirrors the canonical telegram-protocol api/lib blobs byte-identically.

## 6. Target state

This responsibility is tracked as understood/mapped but blocked on a real canonical TLS camouflage transport/generator owner. Current round disposition is **no production delta**: no dead TL parser/serializer and no fake shipping entrypoint.

## 7. Architecture and ownership boundaries

Candidate lifecycle owner: existing canonical `native/telegram-network` transport layer, extended source-neutrally when TLS camouflage is actually implemented.

Rejected current owner for production closure: `native/telegram-protocol` alone. It can model TL framing, but the fixed-U family is an in-memory generator template and has no independent shipping wire lifecycle; adding standalone encode/decode APIs would not close a product responsibility.

## 8. Interfaces / contracts / schemas / data flow

Fixed-U source flow:

`mtproto.tl generated types -> PrepareClientHelloRules() local AST -> Generator::Part::writeBlock -> TLS ClientHello bytes -> TLS socket lifecycle`.

There is no `TlsClientHello/TlsBlock -> MTProto network wire` path to reproduce independently.

Authored source contract for this round (not executed):
- the full family remains adjacent to `help.configSimple` in fixed-U `mtproto.tl`;
- fixed-U `mtproto_tls_socket.cpp` constructs and consumes the family locally;
- canonical/desktop telegram-protocol currently expose no TLS-template API;
- production code remains unchanged until a real TLS camouflage generator owner is present.

## 9. Constraints and non-functional requirements

No custom cryptography, no hidden runner, no CI/build/test execution, no duplicate provider/runtime, and no source-specific product root.

## 10. Failure modes and edge cases

- False closure by adding an unused TL serializer/parser.
- Treating generated TL constructor identity as proof of a network wire responsibility.
- Moving randomness/key generation/HMAC/permutation semantics into telegram-protocol.
- Claiming implemented without a shipping transport entrypoint.

## 11. Implementation strategy

Round 66 is documentation/bookkeeping only. Keep canonical and desktop production blobs unchanged. Track the missing TLS camouflage generator lifecycle as a blocker and continue other independent migration work.

## 12. Verification / test strategy

Stage A: authored source-level contract only; do not run tests, build, lint, generators, validators, CI, packaging, deployment, or runtime acceptance.

Stage B, after the real owner exists: add unit/property contracts for template interpretation and packaged transport/TLS functional evidence bound to the frozen candidate.

## 13. Acceptance criteria / Definition of Done

- AC-1: fixed-U source identities are recorded.
- AC-2: local-template vs MTProto-wire boundary is explicit.
- AC-3: existing owner candidates/rejection are explicit.
- AC-4: no dead production API is introduced.
- AC-5: desktop/canonical provider production blobs remain unchanged.
- AC-6: ledger/P0/source-closure record the blocked mapped responsibility.
- AC-7: executable validation remains not-run in Stage A.

## 14. Release / migration / rollback

N/A for this no-production-delta disposition. Revert the documentation/bookkeeping commit if the source classification is later disproven.

## 15. Observability / evidence

GitHub immutable commit/blob identities plus desktop parity-ledger/P0/source-closure evidence. No runtime evidence is claimed.

## 16. References / provenance

- `telegramdesktop/tdesktop@f985723b552b30fe1c17d377d57f44e61a15a8c4`
- `Telegram/SourceFiles/mtproto/scheme/mtproto.tl@76225965bfc794388db241f3f760db8ab79e84f8`
- `Telegram/SourceFiles/mtproto/details/mtproto_tls_socket.cpp@e7562e3ca1a1da03f5d368085d5e61d887ab3f16`

## 17. Spec compliance record

| Requirement / AC | Status | Evidence / reason |
| --- | --- | --- |
| R1-R6 | passed | Source/bookkeeping boundary defined in this spec; no executable validation required for the disposition itself. |
| AC-1-AC-5 | passed | Fixed-U and live canonical source inspection; no production delta. |
| AC-6 | blocked | Desktop bookkeeping sync is performed after this spec commit and must reference the resulting immutable SHA. |
| AC-7 | passed | Stage A explicitly forbids executable validation; none is run in this round. |
