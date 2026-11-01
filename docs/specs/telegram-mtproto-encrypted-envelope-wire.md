# Telegram MTProto encrypted envelope wire responsibility

Stage: A (implementation only; executable validation deferred)
Fixed upstream U: `telegramdesktop/tdesktop@f985723b552b30fe1c17d377d57f44e61a15a8c4`
Responsibility: `TDRP-R9-MTPROTO-ENCRYPTED-ENVELOPE-WIRE-001`

## Fixed-U source authority

This responsibility is the source-neutral encrypted MTProto outer wire envelope immediately adjacent to the already-closed plaintext handshake envelope.

Fixed-U anchors:

- `Telegram/SourceFiles/mtproto/connection_abstract.cpp@3baabf3f72c7c7edba3151814681fbf167a5ca0a`: `AbstractConnection::prepareSecurePacket` reserves and writes exactly two 32-bit words of `auth_key_id` followed by four 32-bit words of `msg_key`, then the caller appends encrypted data.
- `Telegram/SourceFiles/mtproto/session_private.cpp@13cbc76ec723386027b5b4dcac79999b791c4614`: `SessionPrivate::sendSecureRequest` derives the message key, calls `prepareSecurePacket(_keyId, msgKey, fullSize)`, AES-IGE encrypts after that prefix, and sends the resulting packet. `handleReceived` treats the first six 32-bit words as the same external header, rejects the wrong auth-key id, reads the 128-bit message key, decrypts the remaining block-aligned ciphertext and then validates authenticated plaintext.
- `Telegram/SourceFiles/mtproto/scheme/mtproto.tl@76225965bfc794388db241f3f760db8ab79e84f8`: the outer encrypted envelope is intentionally not a TL constructor; system-message bodies begin only after this manually framed MTProto transport/crypto boundary.

The already-closed plaintext handshake envelope, auth-key binding/persistence, connection selection/scheduling/retry, arbitrary request history, gzip and complete difference decoding are outside this responsibility.

## Existing owner and disposition

Existing owner: `native/telegram-protocol/src/crypto.rs`.

Current canonical blob before this responsibility is recorded: `crypto.rs@7882db6b9f3f5e9643fb963526ccc21dd0862d73`.

Existing `EncryptedEnvelope` already owns the exact source-neutral outer representation:

1. `auth_key_id: u64` serialized little-endian at bytes `0..8`;
2. `message_key: [u8;16]` copied verbatim at bytes `8..24`;
3. `encrypted_data` begins at byte 24 and must be AES-block aligned before cryptographic processing;
4. `encrypt_message` derives `auth_key_id` from the established 256-byte authorization key and returns this envelope;
5. `decrypt_message` rejects auth-key mismatch, empty/non-block-aligned ciphertext, message-key authentication failure, invalid internal plaintext/session/message-id semantics;
6. shipping `native/telegram-network/src/lib.rs@54f509474391bb5118ba4678ba5c566ab3e74ec8` sends `encrypt_message(...).to_bytes()` and receives through `EncryptedEnvelope::from_bytes` followed by `decrypt_message`.

Therefore the production disposition is **no-production-delta**: no parallel Telegram transport/crypto owner and no source-specific wrapper is introduced.

## Exact contract

R1. Outbound bytes are exactly `LE64(auth_key_id) || msg_key[16] || encrypted_data`.

R2. Inbound structural decoding requires at least the 24-byte external header and requires the remaining encrypted byte count to be divisible by the 16-byte AES block size.

R3. Structural decoding preserves all 16 message-key bytes exactly and interprets only the authorization-key id as little-endian integer data.

R4. Shipping receive must fail closed before body dispatch when the authorization-key id does not match the active key, ciphertext is empty/non-block-aligned, authenticated message-key recomputation fails, or the decrypted internal message is structurally invalid.

R5. This responsibility does not own server-salt adoption, session-id lifecycle, message-id history/replay windows, sequence-number policy, retry/scheduling, RPC correlation, gzip, or difference decoding; those remain separate existing owners/responsibilities.

R6. Focused contracts must pin exact byte offsets/endianness, round-trip structural parsing, malformed-short and malformed-block-alignment rejection, plus shipping cryptographic rejection for an empty encrypted payload. Contracts are authored in Stage A and not run.

## Stage-A acceptance boundary

Source-level closure requires:

- the existing canonical owner remains the only production owner;
- focused authored contracts are committed but not executed;
- the desktop provider mirror uses the same authored contract where that mirror exists, without manufacturing a duplicate `crypto.rs`;
- the unique desktop parity ledger records `implemented / coverage_disposition=no-production-delta / validation_status=not-run / accepted=false`;
- P0/source-closure/evidence record the fixed-U anchors, canonical blobs and remote commit readback;
- no GitHub Actions/status/check query or trigger, build, typecheck, lint, schema validator, generator, unit/contract/integration/E2E execution, package, deployment, installation or runtime acceptance is performed.

This slice is source-level wire responsibility closure only. It is not executable verification, candidate readiness, release acceptance, or closure of the broader migration.


## Round 102 source-level compliance readback

Desktop bookkeeping checkpoint is `a6541989393bb6d1e44e76c542192af0cef2c3c0`. Unique responsibility `TDRP-R9-MTPROTO-ENCRYPTED-ENVELOPE-WIRE-001` is recorded as `implemented / coverage_disposition=no-production-delta / validation_status=not-run / accepted=false`.

| Requirement | Source-level status | Evidence / reason |
| --- | --- | --- |
| R1 | passed | Existing canonical `EncryptedEnvelope::to_bytes` in `crypto.rs@7882db6b9f3f5e9643fb963526ccc21dd0862d73` emits exactly LE64 auth_key_id, 16 message-key bytes, then encrypted data. |
| R2 | passed | Existing `EncryptedEnvelope::from_bytes` requires the 24-byte external header and AES-block alignment for all bytes following it. |
| R3 | passed | Structural decoding preserves all 16 message-key bytes and decodes only auth_key_id as little-endian integer data. |
| R4 | passed at source level | Existing `decrypt_message` rejects active-key mismatch, empty/non-block-aligned ciphertext, authenticated message-key mismatch and invalid decrypted plaintext before returning a body. |
| R5 | passed | Shipping `EstablishedSession` remains `telegram-network/src/lib.rs@54f509474391bb5118ba4678ba5c566ab3e74ec8`; no connection scheduler, request-history, binding/persistence, gzip or difference owner was added. |
| R6 | authored / not run | Focused contracts are `protocol_contract.rs@b82069d5b7f42919e087efe053549ebddbe6e278` in canonical and desktop provider test paths. They pin exact offsets/endianness, round-trip structural parsing, short/non-aligned rejection and empty-ciphertext cryptographic rejection. |

Desktop provider bookkeeping preserves network blob `54f509474391bb5118ba4678ba5c566ab3e74ec8` and mirrors the authored contract at commit `e940ec506ea2909dc0f550d533c04f3af7b70d40`. The sparse desktop overlay has no duplicate `crypto.rs`; canonical crypto ownership remains here.

Coverage at desktop checkpoint is `responsibilities_total=103, unknown=16172, unread=16145, omitted=0, unmapped_responsibilities=0`.

Source-level compliance is complete for this isolated encrypted-envelope wire slice. This is not executable verification, candidate readiness, release acceptance, or closure of the broader migration. Auth-key binding/persistence/expiry/renewal, broader connection scheduling/racing/send retry, arbitrary request histories, gzip, complete difference/differenceSlice decoding/application, broader source/legal closure, Stage-B evidence and independent release ACCEPT remain open. No GitHub Actions/status/check query or trigger, build, typecheck, lint, schema validator, generator, unit/contract/integration/E2E execution, package, deployment, installation or runtime acceptance was performed.
