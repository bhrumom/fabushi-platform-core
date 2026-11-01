# Telegram MTProto quick-ACK wire boundary — Specification

Status: active
Owner: shared telegram-protocol / telegram-network
Last updated: 2026-11-01
Related issue/task/PR: TDRP-R9-MTPROTO-QUICK-ACK-WIRE-001

## 1. Context / problem

Fixed upstream authority remains `telegramdesktop/tdesktop@f985723b552b30fe1c17d377d57f44e61a15a8c4`.

The adjacent fixed-U MTProto code contains no live quick-ACK lifecycle. `TcpConnection::Protocol::Version0::finalizePacket` emits only canonical abridged length prefixes and never sets a quick-ACK request bit. `VersionD::finalizePacket` emits only its LE32 byte length. On receive, `TcpConnection::parsePacket/socketPacket` explicitly comments that the newer quick-ACK form is not supported and routes short packets only as nop/error. `SessionPrivate::sendSecureRequest` computes the MTProto 2 SHA-256 digest used for the message key, but fixed-U does not turn that adjacent digest into a quick-ACK request/correlation lifecycle.

Fabushi already has source-neutral generic quick-ACK wire primitives in `native/telegram-protocol`: `quick_ack_token`, `TransportFrameCodec::encode_requesting_quick_ack`, and Abridged/Intermediate token recognition. The shipping `telegram-network::TcpTransport` never requests quick-ACK, but its receive loop currently silently discards a decoded token. Silent discard has no fixed-U owner and can hide an unsolicited control token even though no request-history correlation owner exists.

## 2. Goal

Close the exact source-level quick-ACK wire boundary without introducing arbitrary request history:

- retain the existing history-free token/marker codec in the canonical `telegram-protocol` owner;
- keep fixed-U-equivalent shipping sends free of quick-ACK request markers;
- make the shipping network boundary fail closed if a quick-ACK token is received without a correlation lifecycle;
- leave request-to-token correlation explicitly blocked/open.

## 3. Non-goals / out of scope

- enabling quick-ACK requests in the shipping Telegram connection;
- correlating a received quick-ACK token to `haveSent/_resendingIds/toSend/_sentContainers` or any new request registry;
- changing MTProto message-id, ack, resend, scheduler, retry, reconnect, proxy/TLS, auth-key binding/persistence, gzip, or difference decoding/application;
- changing padded-intermediate quick-ACK behavior already closed as unsupported;
- executable validation in Stage A.

## 4. Requirements

- R1: Existing `telegram-protocol::quick_ack_token` remains a pure deterministic wire helper with no request-history dependency.
- R2: Existing Abridged/Intermediate request-marker and response-token codec behavior remains isolated in `telegram-protocol::TransportFrameCodec`; no second codec/owner is introduced.
- R3: Fixed-U-equivalent shipping `TcpTransport::send_payload` must continue to call ordinary `encode`, never `encode_requesting_quick_ack`.
- R4: If `TcpTransport::receive_payload` decodes a quick-ACK token, it must fail closed with a typed network error instead of silently discarding the token or pretending correlation succeeded.
- R5: No request-history/correlation state, callback settlement, resend mutation, or scheduling side effect may be added by this responsibility.
- R6: `PaddedIntermediate` remains `QuickAckUnsupported`; this responsibility does not reopen VersionD framing.
- R7: Authored-only contracts must pin (a) normal abridged shipping sends with the high request bit clear and (b) unsolicited abridged quick-ACK receipt producing the typed fail-closed network error.
- R8: Stage A must not run build, typecheck, lint, generator, schema validation, unit/contract/integration/E2E, packaging, deployment, runtime acceptance, or GitHub Actions/status/check queries.

## 5. Current state

Canonical protocol code already owns pure quick-ACK token/marker representation. Canonical network code does not request quick-ACK, but `receive_payload` currently loops past `frame.quick_ack_token` and discards it.

## 6. Target state

No protocol ownership split. The protocol codec remains unchanged unless source review finds a concrete wire defect. The existing shipping network boundary rejects an unsolicited/uncorrelated quick-ACK token with a typed error. Correlation remains absent and explicitly blocked.

## 7. Architecture and ownership boundaries

- `native/telegram-protocol/src/crypto.rs`: pure quick-ACK token derivation helper; no lifecycle state.
- `native/telegram-protocol/src/transport.rs`: pure request marker / token recognition codec; no socket or history state.
- `native/telegram-network/src/lib.rs`: shipping socket boundary; ordinary sends only; fail-closed disposition for received quick-ACK tokens.
- Desktop provider mirrors canonical network/test files; no second Telegram runtime/provider is created.

## 8. Interfaces / contracts / schemas / data flow

Normal shipping send:
`EstablishedSession/TcpTransport -> TransportFrameCodec::encode -> wire`.

Unexpected quick-ACK receive:
`wire -> TransportFrameCodec::decode -> DecodedFrame.quick_ack_token=Some(token) -> NetworkError::UnexpectedQuickAck(token)`.

No token-to-request mapping is performed.

## 9. Constraints and non-functional requirements

Existing-owner-first, source-neutral Rust, no new dependency, no request-history store, no scheduler change, no executable validation in Stage A.

## 10. Failure modes and edge cases

- An unsolicited quick-ACK token is silently ignored: forbidden; must fail closed.
- A shipping send accidentally sets the quick-ACK request marker: forbidden.
- A token is used to settle the current RPC/ping without an explicit correlation owner: forbidden.
- Generic protocol marker/token helpers are deleted merely because fixed-U does not use them: forbidden; keep them isolated and non-shipping.
- Padded-intermediate begins accepting quick-ACK: forbidden.

## 11. Implementation strategy

1. Preserve existing `telegram-protocol` quick-ACK wire primitives.
2. Add a typed `NetworkError::UnexpectedQuickAck(u32)`.
3. Change only `TcpTransport::receive_payload` so decoded quick-ACK tokens return that error.
4. Author focused network contracts; do not run them.
5. Mirror canonical network/test blobs into the desktop provider and update the single parity ledger/P0/source-closure/evidence.

## 12. Verification / test strategy

Authored only in Stage A:

- assert the existing abridged shipping request length byte has bit 7 clear;
- inject a four-byte abridged quick-ACK token and assert `UnexpectedQuickAck(token)`;
- rely on the already-authored canonical protocol contracts for generic Abridged/Intermediate marker/token behavior and the existing known-vector `quick_ack_token` derivation;
- do not execute any test or validator.

## 13. Acceptance criteria / Definition of Done

- AC-1: Durable spec precedes behavior change.
- AC-2: No new owner; protocol wire primitives remain in existing `telegram-protocol`.
- AC-3: Shipping sends do not request quick-ACK.
- AC-4: Shipping receive fails closed on a decoded quick-ACK token.
- AC-5: Correlation/history remains explicitly blocked/open.
- AC-6: Focused contracts are authored and mirrored but not executed.
- AC-7: Desktop unique ledger records source-level implementation with `validation_status=not-run` and `accepted=false`.
- AC-8: P0/source-closure/evidence and remote SHA readback are recorded with non-force expected-SHA CAS.

## 14. Release / migration / rollback

No runtime migration. Rollback is the single network receive-boundary change plus its authored tests/spec bookkeeping. Release validation remains Stage B.

## 15. Observability / evidence

Source-level evidence consists of fixed-U blob identities, canonical protocol/network blob identities, authored-only contract blobs, desktop mirror blobs, ledger/P0/source-closure/evidence commits, and final remote commit readback. No executable evidence is claimed.

## 16. References / provenance

- fixed-U `Telegram/SourceFiles/mtproto/connection_tcp.cpp@d721a9058e1885d2964ded55bd4e4c7c2ce9e381`: Version0/VersionD framing plus `parsePacket/socketPacket` unsupported quick-ACK comments.
- fixed-U `Telegram/SourceFiles/mtproto/connection_abstract.cpp@3baabf3f72c7c7edba3151814681fbf167a5ca0a`: secure packet transport-prefix adjacency.
- fixed-U `Telegram/SourceFiles/mtproto/session_private.cpp@13cbc76ec723386027b5b4dcac79999b791c4614`: `sendSecureRequest` SHA-256/msg-key generation and existing request-history correlation boundaries.
- canonical `native/telegram-protocol/src/crypto.rs@32d69573acc6ef4ce7bcc80073f5cdd7e2bea7ad`.
- canonical `native/telegram-protocol/src/transport.rs@74189488dc3ec1015edc59989f7aced8ad65f5c3`.
- canonical `native/telegram-network/src/lib.rs@3034acab752ca37531033bb8a1d106003b20265d`.

## 17. Spec compliance record

| Requirement / AC | Status | Evidence / reason |
| --- | --- | --- |
| R1-R8 | passed | Source-level Stage-A implementation is recorded below. R8 is satisfied by deferring all executable validation; no executable gate is claimed. |
| AC-1 | passed | Durable spec checkpoint `1a09a5674ec16af4228bbf0aed0fb302cb52fba0` preceded the behavior change. |
| AC-2 | passed | Existing `telegram-protocol` quick-ACK token/marker primitives remain the sole protocol wire owner and are unchanged. |
| AC-3 | passed | Existing shipping `TcpTransport::send_payload` still calls ordinary `TransportFrameCodec::encode`; no quick-ACK request marker is enabled. |
| AC-4 | passed | Production commit `605b8928de5fb64bf235196093ef804e350994f2` makes decoded uncorrelated tokens return `NetworkError::UnexpectedQuickAck(token)`; network blob `7c442299aedd4aa7444ba53c7ab8212542dfc2d1`. |
| AC-5 | passed | No request-history/correlation state, callback settlement, resend mutation or scheduling owner was introduced; correlation remains explicitly open. |
| AC-6 | passed | Authored-only network contracts are `d1cec8b3952a4be1b02c6f1dae1ab008991a0342` / blob `9dfb554bae0dac9ea68ebd6b2fe7cab664cc1240`, mirrored byte-identically to desktop; not run. |
| AC-7 | passed | Desktop bookkeeping `da6bbfcafabb66681ba8614cf2d97c9e9e43ca5f` adds the unique responsibility as `implemented / rewritten / not-run / accepted=false`; ledger blob `f8c27fcbbd782fae1c30a8d9aa313f5917869183`. |
| AC-8 | passed | Desktop provider sync `f24b51ec5ba2d959f8c08fe44c6af706f38d7224`, P0 blob `8d105d05edffa4518c9d086624004e5f5cb6c393`, source-closure blob `a9feff9e8d6646dbc5fb60483bbe3dbc4d4e9c59`, evidence blob `0f692837f19ca4ea30c893a0c73c61b5525bb8b7`; all branch writes used non-force expected-SHA CAS. |

Allowed final statuses: `passed`, `blocked`, `not-applicable`.


## Round 108 implementation record

The isolated quick-ACK wire boundary is source-level closed for Stage A only.

- Fixed-U source: `connection_tcp.cpp@d721a9058e1885d2964ded55bd4e4c7c2ce9e381`, `connection_abstract.cpp@3baabf3f72c7c7edba3151814681fbf167a5ca0a`, `session_private.cpp@13cbc76ec723386027b5b4dcac79999b791c4614`.
- Durable spec-first checkpoint: `1a09a5674ec16af4228bbf0aed0fb302cb52fba0`.
- Canonical production: `605b8928de5fb64bf235196093ef804e350994f2`, network blob `7c442299aedd4aa7444ba53c7ab8212542dfc2d1`.
- Canonical authored-only contracts: `d1cec8b3952a4be1b02c6f1dae1ab008991a0342`, network contract blob `9dfb554bae0dac9ea68ebd6b2fe7cab664cc1240`.
- Desktop provider synchronization: `f24b51ec5ba2d959f8c08fe44c6af706f38d7224`.
- Desktop bookkeeping: `da6bbfcafabb66681ba8614cf2d97c9e9e43ca5f`.
- Unique ledger status: `implemented / coverage_disposition=rewritten / validation_status=not-run / accepted=false`.
- Coverage: `responsibilities_total=109, unknown=16172, unread=16145, omitted=0, unmapped_responsibilities=0`.

Fixed-U shipping never requests quick-ACK, so this responsibility intentionally does not synthesize token-to-request correlation. Existing generic protocol marker/token primitives remain available but non-shipping. An unexpected decoded token now fails closed at the shipping network boundary instead of being silently discarded.

No build, typecheck, lint, schema validator, generator, unit/contract/integration/E2E execution, GitHub Actions/status/check query, package, deployment, installation or runtime acceptance was performed. Executable validation remains `not-run`; acceptance remains false.
