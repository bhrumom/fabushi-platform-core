# Telegram MTProto secure inner message wire responsibility

Stage: A (implementation only; executable validation deferred)
Fixed upstream U: `telegramdesktop/tdesktop@f985723b552b30fe1c17d377d57f44e61a15a8c4`
Responsibility: `TDRP-R9-MTPROTO-SECURE-INNER-MESSAGE-WIRE-001`

## Fixed-U source authority

This responsibility is the authenticated plaintext structure inside the already-closed encrypted MTProto outer envelope.

Fixed-U anchors:

- `Telegram/SourceFiles/mtproto/session_private.cpp@13cbc76ec723386027b5b4dcac79999b791c4614`: `SessionPrivate::sendSecureRequest` writes `server_salt` and `session_id` before the existing message id / sequence number / message length / body region, adds MTProto 2 padding, hashes the complete plaintext and encrypts it. `handleReceived` decrypts the same structure as 8-byte salt, 8-byte session, 8-byte message id, 4-byte sequence number, 4-byte message length, body, then 12..1024 bytes of padding; it rejects body lengths above 16 MiB, non-4-byte body lengths, invalid padding, session mismatch and invalid inbound message-id parity before body dispatch.
- `Telegram/SourceFiles/mtproto/connection_abstract.cpp@3baabf3f72c7c7edba3151814681fbf167a5ca0a`: the external `auth_key_id || msg_key` framing is separate and already closed.
- `Telegram/SourceFiles/mtproto/scheme/mtproto.tl@76225965bfc794388db241f3f760db8ab79e84f8`: this secure inner message is manually framed and is not a TL constructor.

The already-closed plaintext/encrypted outer envelopes, handshake request/result responsibilities, auth-key binding/persistence, connection scheduling/racing/retry, arbitrary request history, gzip and complete difference decoding/application are outside this responsibility.

## Existing owner and intended disposition

Existing owner: `native/telegram-protocol/src/crypto.rs`, consumed by `native/telegram-network/src/lib.rs`.

Current canonical `PlainMessage` already owns the fixed 32-byte internal header, body and padding; `encrypt_message` and `decrypt_message` are already in the shipping `EstablishedSession` path.

Source review finds one exact fixed-U gap inside this owner: `validate_plain_message` currently has no 16 MiB body-length ceiling although fixed-U rejects `messageLength > 16 * 1024 * 1024`. The production disposition is therefore **minimal production delta in the existing owner**: add only that body-length guard. No parallel Telegram crypto/session owner is introduced.

## Exact contract

R1. Plaintext bytes are exactly `LE64(server_salt) || LE64(session_id) || LE64(message_id) || LE32(seq_no) || LE32(msg_len) || body || padding`.

R2. `msg_len` is the body byte count, must be positive, divisible by four, and must not exceed 16 MiB.

R3. Padding is the remaining authenticated plaintext after `msg_len` bytes, must contain 12..1024 bytes, and the complete plaintext length must be divisible by the 16-byte AES block size.

R4. Shipping inbound decoding must preserve all fixed-width fields exactly, reject truncated/overflowing body boundaries, reject session mismatch, and accept only odd server message ids (the fixed-U `msg_id & 3 == 1 || 3` rule).

R5. Shipping outbound encoding must continue to require client-to-server message ids divisible by four when cryptographic direction validation applies and must fail closed on invalid body/padding structure before encryption.

R6. Server-salt adoption, replay-window/history registration, ACK scheduling, request correlation, retry/backoff and connection lifecycle are separate responsibilities and are not moved into `telegram-protocol`.

R7. Focused contracts must pin exact internal byte offsets/endianness, authenticated round-trip decoding, wrong-session rejection, wrong-direction message-id rejection and the fixed-U 16 MiB body ceiling. Contracts are authored in Stage A and not run.

## Stage-A acceptance boundary

Source-level closure requires:

- durable spec precedes behavior change;
- the canonical shared `telegram-protocol` owner remains the only production owner;
- only the fixed-U body ceiling is added if review confirms all other rules already match;
- shipping `telegram-network` continues to call `encrypt_message` / `decrypt_message` without a duplicate crypto implementation;
- focused canonical and desktop-mirror contracts are committed but not executed;
- the unique desktop parity ledger records `implemented / coverage_disposition=rewritten / validation_status=not-run / accepted=false`;
- P0/source-closure/evidence record fixed-U anchors, canonical blobs and remote commit readback;
- no GitHub Actions/status/check query or trigger, build, typecheck, lint, schema validator, generator, unit/contract/integration/E2E execution, package, deployment, installation or runtime acceptance is performed.

This slice is source-level wire responsibility closure only. It is not executable verification, candidate readiness, release acceptance, or broader migration closure.
