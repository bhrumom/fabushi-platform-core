# Telegram MTProto plaintext auth-key handshake envelope wire contract

Status: Stage A implementation contract; executable validation deferred.

## Fixed-U provenance

- Upstream: `telegramdesktop/tdesktop@f985723b552b30fe1c17d377d57f44e61a15a8c4`.
- `Telegram/SourceFiles/mtproto/scheme/mtproto.tl@76225965bfc794388db241f3f760db8ab79e84f8` defines the auth-key handshake request/response bodies already carried by the current `AuthKeyHandshake` owner.
- `Telegram/SourceFiles/mtproto/details/mtproto_dc_key_creator.cpp@13c57454f14b814b5923ea253048fb881ad8cf03` sends every req_pq_multi / req_DH_params / set_client_DH_params handshake step through `DcKeyCreator::sendNotSecureRequest -> AbstractConnection::prepareNotSecurePacket`.
- `Telegram/SourceFiles/mtproto/connection_abstract.h@217b684fa222c42e968dfddeaa62f16a0054017f` writes the not-secure envelope after transport framing as zero `auth_key_id:long`, `message_id:long`, `message_data_length:int`, then the TL request plus 0..63 random 32-bit padding words; the length includes request plus padding.
- `Telegram/SourceFiles/mtproto/connection_abstract.cpp@3baabf3f72c7c7edba3151814681fbf167a5ca0a` validates zero auth key id, server-message-id low bits, positive 4-byte-aligned declared length, and bounds the declared body before handing it to the handshake parser.
- Existing Fabushi shipping path: `native/telegram-network/src/lib.rs` `establish_auth_key -> exchange_and_validate -> TcpTransport::exchange_plaintext -> PlaintextEnvelope::encode/decode`.

## Goal

Close the fixed-U unencrypted auth-key handshake envelope as its own exact wire responsibility in the existing telegram-protocol owner and keep the existing telegram-network caller as the shipping consumer. Add the fixed-U outbound random-padding behavior and fail-closed envelope length/alignment validation without moving connection scheduling or auth-key lifecycle ownership.

## Non-goals

This slice does not own TCP/HTTP/TLS connection selection, socket retry/racing, message-id generation policy, request correlation beyond the existing one-handshake exchange, temporary/permanent auth-key binding or persistence, session persistence, arbitrary request history, gzip, or complete difference/differenceSlice decoding/application. It does not create a second handshake or transport owner.

## Requirements

1. Keep `PlaintextEnvelope` as the unique canonical unencrypted handshake-envelope owner.
2. Wire layout is exactly little-endian: `auth_key_id=0:i64`, `message_id:i64`, `message_data_length:i32`, then declared message data.
3. Outbound handshake message data consists of the existing TL body plus caller-supplied random padding; padding length must be 0..252 bytes and divisible by 4, matching fixed-U 0..63 padding words.
4. The encoded `message_data_length` includes both TL body and padding. The existing no-padding `encode()` remains a compatibility wrapper over the exact encoder with empty padding.
5. The encoder fails closed for empty or non-4-byte-aligned TL bodies, invalid padding length/alignment, body/padding size overflow, or the existing configured plaintext maximum.
6. The decoder rejects non-zero `auth_key_id`, negative/zero/non-4-byte-aligned declared length, declared length above the configured maximum, truncation, and any payload length mismatch. It returns exactly the declared message-data bytes.
7. `TcpTransport::exchange_plaintext` remains the real shipping caller and adds fixed-U-equivalent secure random padding: one secure random selector byte masked with `0x3f`, multiplied by four, then that many secure random padding bytes; no connection scheduling behavior changes.
8. Existing `MessageIdGuard::validate_server_message_id` remains the owner of server message-id validity after envelope decode; envelope code does not duplicate clock/window policy.
9. Existing auth-key handshake phase serializers/parsers and state transitions remain unchanged except for envelope framing.
10. Focused Stage-A contracts are authored but not executed for exact no-padding layout, padded length/layout, invalid body alignment/emptiness, invalid padding size/alignment, non-zero auth key id, invalid declared length, truncation/mismatch, and maximum bounds.
11. Desktop synchronization mirrors only already-tracked telegram-network and contract artifacts; no nonexistent desktop `handshake.rs` duplicate is created.
12. The unique desktop parity ledger, P0/source-closure and round evidence record `implemented / validation_status=not-run / accepted=false`.
13. Preserve Stage A zero-CI: no Actions/status/check query or trigger, build, typecheck, lint, schema validator, generator, unit/contract/integration/E2E execution, package, deployment, installation or runtime acceptance.

## Ownership boundary

- `native/telegram-protocol/src/handshake.rs`: envelope structure, exact encode/decode validation, fixed-U padding boundary.
- `native/telegram-network/src/lib.rs`: secure-random padding bytes and existing socket exchange only.
- `MessageIdGuard`: server message-id semantic/time validation.
- No owner is added for binding/persistence, connection scheduling, arbitrary historical requests, gzip, or complete updates difference.

## Verification strategy

Stage A only authors source contracts and performs manual source/readback review. No executable validation runs. Stage B must compile and run the focused protocol/network contracts plus the broader campaign against the frozen candidate.

## Acceptance

Source-level acceptance for this slice requires remote readback of the durable spec, canonical production edits, authored-only contracts, synchronized desktop tracked mirrors, unique ledger/P0/source-closure/evidence bookkeeping, and a final compliance note. Executable verification remains `not-run`; `accepted` remains false.

## Fail-closed boundary

Malformed envelope lengths, misaligned/empty bodies, invalid padding boundaries, non-zero auth key ids, truncation and trailing/missing bytes are rejected before any handshake body parser is invoked. Broader lifecycle and transport scheduling remain outside this responsibility.


## Round 101 source-level compliance readback

Desktop bookkeeping checkpoint is `0d8b5b19d9bab4b509db550f8b6ce57441b80ed5`. Unique responsibility `TDRP-R9-MTPROTO-PLAINTEXT-HANDSHAKE-ENVELOPE-WIRE-001` is recorded as `implemented / coverage_disposition=rewritten / validation_status=not-run / accepted=false`.

| Requirement | Source-level status | Evidence / reason |
| --- | --- | --- |
| R1 | passed | Existing canonical `PlaintextEnvelope` remains the unique unencrypted auth-key handshake envelope owner in `handshake.rs@301554c60fe318a621726cacdd37f0d5d1de2a3e`. |
| R2 | passed | `encode_with_padding` emits little-endian zero `auth_key_id`, caller `message_id`, inclusive i32 `message_data_length`, then message data. |
| R3 | passed | Canonical padding guard permits only 0..252 bytes divisible by four, matching fixed-U 0..63 32-bit words. |
| R4 | passed | Existing `encode()` is the empty-padding compatibility path; padded encoding includes TL body plus padding in the declared length. |
| R5 | passed at source level | Outbound empty/misaligned bodies, invalid padding, overflow and configured maximum violations fail closed before bytes are returned. |
| R6 | passed at source level | Decoder rejects non-zero auth key id, zero/negative/misaligned/oversized length and declared/actual mismatch before returning message data. |
| R7 | passed | Existing shipping `TcpTransport::exchange_plaintext` in `telegram-network/src/lib.rs@54f509474391bb5118ba4678ba5c566ab3e74ec8` now uses secure random fixed-U-equivalent padding without changing connection selection/retry. |
| R8 | passed | Existing `exchange_and_validate -> MessageIdGuard::validate_server_message_id` remains the server message-id semantic/time owner. |
| R9 | passed | Existing req_pq_multi / req_DH_params / set_client_DH_params body serializers/parsers and AuthKeyHandshake phase state remain otherwise unchanged. |
| R10 | authored / not run | Focused contracts are in `protocol_contract.rs@a7c10e88172ccd664bad0370acea8fac7a3e2f3d`; no executable validation was run. |
| R11 | passed | Desktop existing telegram-network and contract mirrors are byte-identical to canonical at bookkeeping checkpoint `0d8b5b19d9bab4b509db550f8b6ce57441b80ed5`; no duplicate desktop handshake.rs was created. |
| R12 | passed | Unique desktop ledger/P0/source-closure/evidence record coverage `responsibilities_total=102, unknown=16172, unread=16145, omitted=0, unmapped_responsibilities=0` with validation not-run and accepted false. |
| R13 | passed | No GitHub Actions/status/check query or trigger, build, typecheck, lint, schema validator, generator, unit/contract/integration/E2E execution, package, deployment, installation or runtime acceptance was performed. |

Source-level compliance is complete for this isolated plaintext-handshake-envelope wire slice. This is not executable verification, candidate readiness, release acceptance, or closure of the broader migration. Auth-key binding/persistence/expiry/renewal, broader connection scheduling/racing/send retry, arbitrary request histories, gzip, complete difference/differenceSlice decoding/application, broader source/legal closure, Stage-B evidence and independent release ACCEPT remain open.
