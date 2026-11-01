# Telegram MTProto abridged transport frame wire responsibility

Stage: A (implementation only; executable validation deferred)
Fixed upstream U: `telegramdesktop/tdesktop@f985723b552b30fe1c17d377d57f44e61a15a8c4`
Responsibility: `TDRP-R9-MTPROTO-ABRIDGED-TRANSPORT-FRAME-WIRE-001`

## Fixed-U source authority

This responsibility is only the MTProto abridged packet-length frame carried after connection setup / transport obfuscation has selected the Version0 framing rules.

Fixed-U anchors:

- `Telegram/SourceFiles/mtproto/connection_tcp.cpp@d721a9058e1885d2964ded55bd4e4c7c2ce9e381`: `TcpConnection::Protocol::Version0::finalizePacket`, `readPacketLength`, and `readPacket` define the exact abridged frame. Payload length is measured in 4-byte words. For 1..126 words the frame uses one length byte. For 127 or more words the frame uses `0x7f` followed by a 24-bit little-endian word count. The decoder rejects the extended representation when that count is below `0x7f`.
- `Telegram/SourceFiles/mtproto/scheme/mtproto.tl@76225965bfc794388db241f3f760db8ab79e84f8`: this transport frame is not a TL constructor.
- `Telegram/SourceFiles/mtproto/session_private.cpp@13cbc76ec723386027b5b4dcac79999b791c4614`: secure-message body construction is separate and already closed.

The connection-start nonce/AES-CTR obfuscation prefix, secret-dependent Version1 key derivation, VersionD padded-intermediate framing, quick-ack behavior, connection racing/scheduling/retry, auth-key binding/persistence, arbitrary request history, gzip, and complete difference decoding/application are outside this responsibility.

## Existing owner and current gap

Existing canonical owner: `native/telegram-protocol/src/transport.rs`, consumed by `native/telegram-network/src/lib.rs`.

The shipping `TcpTransport` already uses `TransportFrameCodec`; `NetworkConfig::default()` selects `TransportMode::Abridged`. Existing encode logic already writes the fixed-U one-byte and `0x7f + 3-byte LE` forms and rejects non-word-aligned payloads.

Source review finds one exact fixed-U decode gap: `decode_abridged` currently accepts the four-byte extended form even when the decoded word count is below 127, while fixed-U `Version0::readPacketLength` rejects that non-canonical representation. Production disposition is therefore a minimal fail-closed change in the existing owner only.

## Exact contract

R1. Abridged payloads are non-empty and their byte length is divisible by four.

R2. Payloads of 1..126 words use exactly one unsigned length byte equal to the word count.

R3. Payloads of 127 or more words use exactly `0x7f || word_count_le24`. The 24-bit count must be at least 127; extended encodings of smaller counts are invalid.

R4. The decoder treats an incomplete short/extended header or incomplete payload as incomplete input rather than fabricating a frame. A decoded payload remains subject to the existing configured maximum frame bound.

R5. The existing shipping `telegram-network::TcpTransport` remains the only socket owner and continues to call `TransportFrameCodec::encode/decode`; no second transport implementation is introduced.

R6. This slice does not claim closure of the fixed-U 64-byte connection-start obfuscation/AES-CTR prefix, Version1/VersionD secret handling, or quick-ack semantics. Those remain separate responsibilities.

R7. Focused contracts must pin the 126-word short boundary, 127-word canonical extended boundary, rejection of a non-canonical extended 126-word length, and fragmented extended-header behavior. They are authored in Stage A and not run.

## Stage-A acceptance boundary

Source-level closure requires:

- this durable spec precedes behavior change;
- the canonical shared `telegram-protocol::transport` owner remains the only framing owner;
- only the non-canonical extended-length decode gap is changed if all other Version0 frame rules match;
- shipping `telegram-network` stays wired to the same codec without connection-scheduler changes;
- focused canonical and desktop-mirror contracts are committed but not executed;
- the unique desktop parity ledger records `implemented / coverage_disposition=rewritten / validation_status=not-run / accepted=false`;
- P0/source-closure/evidence record fixed-U anchors, canonical blobs, and remote commit readback;
- no GitHub Actions/status/check query or trigger, build, typecheck, lint, schema validator, generator, unit/contract/integration/E2E execution, package, deployment, installation, or runtime acceptance is performed.

This slice is source-level wire responsibility closure only. It is not executable verification, candidate readiness, release acceptance, or broader connection-transport closure.
