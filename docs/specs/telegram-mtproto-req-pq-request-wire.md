# Telegram MTProto req_pq request wire contract

Status: Stage A implementation contract; executable validation deferred.

## Fixed-U provenance

- Upstream: `telegramdesktop/tdesktop@f985723b552b30fe1c17d377d57f44e61a15a8c4`.
- `Telegram/SourceFiles/mtproto/scheme/mtproto.tl@76225965bfc794388db241f3f760db8ab79e84f8` defines:
  - `req_pq#60469778 nonce:int128 = ResPQ`.
  - Adjacent `req_pq_multi#be7e8ef1 nonce:int128 = ResPQ` is a distinct request and is not serialized by this builder.
- `Telegram/SourceFiles/mtproto/connection_abstract.cpp@3baabf3f72c7c7edba3151814681fbf167a5ca0a` shows `AbstractConnection::preparePQFake` constructing `MTPReq_pq(nonce)` before wrapping it in a not-secure packet with a generated MTProto message id.
- `Telegram/SourceFiles/mtproto/session_private.cpp@13cbc76ec723386027b5b4dcac79999b791c4614` was reread as adjacent session context and contains no `req_pq` serializer or lifecycle branch.

## Scope and ownership

This slice adds only the exact `req_pq` TL request serializer to the existing canonical Rust `telegram-protocol` owner and the byte-identical desktop provider copy. It does not implement or claim the connection-probe scheduler, TCP/HTTP/IPv4/IPv6 racing, nonce generation, not-secure packet framing, MTProto message-id generation, `ResPQ` parsing/application, auth-key creation state, retry/backoff, persistence, or session/transport lifecycle.

## Requirements

- R1: Export `REQ_PQ_CONSTRUCTOR = 0x60469778`.
- R2: Export `build_req_pq(nonce: &[u8; 16]) -> Vec<u8>`.
- R3: The builder emits exactly 20 bytes: four little-endian constructor bytes followed by the 16 nonce bytes exactly as supplied by the caller.
- R4: The builder performs no byte reversal, integer reinterpretation, allocation-dependent padding, TL string/bytes framing, lookup, randomness, network, persistence, scheduler, callback, ACK/resend/container, gzip, or updates-difference side effect.
- R5: `req_pq_multi`, connection-probe orchestration, not-secure envelope/message-id construction, `ResPQ` parsing/application, and auth-key lifecycle remain separate/fail-closed until independently mapped to exact owners.
- R6: Stage-A contracts are authored but not executed.

## Authored contracts

- exact constructor + raw 16-byte nonce layout;
- nonce identity is preserved for all byte values, including all-zero and boundary-pattern fixtures;
- public re-export through the existing protocol facade;
- desktop provider source remains byte-identical to canonical source after synchronization.

## Acceptance

Source-level implementation may advance to `implemented / validation_status=not-run / accepted=false`. No build, typecheck, lint, schema validator, generator, unit/contract/integration/E2E, package, deployment, installation, GitHub Actions, checks, or statuses are executed or queried in Stage A.
