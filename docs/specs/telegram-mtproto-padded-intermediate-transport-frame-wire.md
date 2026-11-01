# Telegram MTProto padded-intermediate transport frame wire

Responsibility: `TDRP-R9-MTPROTO-PADDED-INTERMEDIATE-TRANSPORT-FRAME-WIRE-001`

Stage: A source-level migration only. Validation remains `not-run`; acceptance remains false.

## Fixed source authority

- `telegramdesktop/tdesktop@f985723b552b30fe1c17d377d57f44e61a15a8c4`
- `Telegram/SourceFiles/mtproto/connection_tcp.cpp@d721a9058e1885d2964ded55bd4e4c7c2ce9e381`
- Symbols: `TcpConnection::Protocol::VersionD::finalizePacket`, `readPacketLength`, `readPacket`.
- `VersionD::id() == 0xDDDDDDDD` and VersionD secret selection / SHA-256 key derivation are adjacent but explicitly outside this responsibility.

## Exact isolated wire contract

This responsibility owns only the plaintext frame bytes immediately before/after the connection-level AES-CTR transform.

1. The frame starts with a four-byte little-endian unsigned byte count for the bytes that follow.
2. The unpadded MTProto payload is a non-empty sequence of 32-bit words and therefore is at least four bytes and divisible by four.
3. Outbound VersionD framing appends 0..15 random padding bytes; the encoded length counts payload plus padding.
4. A deterministic explicit-padding encoder may be retained for fixtures/contracts, but the shipping default encoder for `PaddedIntermediate` must choose the 0..15 padding length and bytes from OS randomness.
5. Receive parsing is incremental: fewer than four header bytes or an incomplete declared body returns “need more data”.
6. Fixed U accepts a declared body only when total packet length `declared_length + 4` is at least 8 and strictly below `0x04000000` bytes.
7. The decoder returns the declared body as an opaque payload-plus-padding byte span; padding separation belongs to the higher MTProto envelope/body-length owner.
8. Fixed-U VersionD does not independently implement the transport quick-ACK request/response forms handled by the generic intermediate codec. This isolated VersionD mode therefore fails closed on quick-ACK request encoding and on high-bit/otherwise out-of-range length headers rather than interpreting them as ACK tokens.

## Existing-owner disposition

Canonical owner remains `native/telegram-protocol/src/transport.rs::TransportFrameCodec`.

Shipping carrier remains `native/telegram-network/src/lib.rs::TcpTransport`; no second transport/runtime/provider is introduced. `TcpTransport` already delegates frame encode/decode to `TransportFrameCodec`.

Required source-level delta is limited to the `PaddedIntermediate` branch:

- random 0..15-byte shipping padding in `encode`;
- exact word alignment for the unpadded payload;
- exact fixed-U VersionD receive length floor/strict ceiling;
- fail-closed VersionD quick-ACK handling;
- preservation of partial-input behavior and configured `max_frame_bytes` as an additional local safety limit.

## Explicit exclusions

This responsibility does not close:

- the 64-byte connection-start nonce/prefix;
- AES-CTR send/receive state;
- Version1 or VersionD secret validation and SHA-256 key derivation;
- placement of protocol/DC identifiers in the obfuscated prefix;
- quick-ACK lifecycle/correlation;
- connection scheduling, racing, retry, proxy selection, or timeout policy;
- auth-key binding/persistence/expiry/renewal;
- arbitrary request history, gzip, or complete updates difference decoding/application.

## Authored-only contract requirements

Contracts must be written but not executed in Stage A and cover:

- explicit padding lengths 0 and 15 with exact LE32 length accounting;
- rejection of non-word-aligned unpadded payload;
- fragmented four-byte header and fragmented declared body;
- rejection of declared body lengths 0..3;
- acceptance at the highest fixed-U legal declared length only as a header/partial-input boundary without allocating the body;
- rejection when `declared_length + 4 >= 0x04000000`;
- VersionD quick-ACK request rejection and rejection of high-bit length-as-token interpretation;
- preservation of generic `Intermediate` quick-ACK behavior outside VersionD.

No build, typecheck, lint, generator, schema validator, test, package, deployment, installation, runtime acceptance, Actions/status/check query, or CI trigger is permitted in this Stage-A slice.


## Round 105 implementation record

Source-level production implementation is `3a3d850139082e32a783b7d03b6ea2fd31b00ee9`.

- Canonical `native/telegram-protocol/src/transport.rs@5d6abd61f130b25baae10a6269eebd20fd025b3a` keeps the existing `TransportFrameCodec` owner and changes only the padded-intermediate branch: shipping `encode` uses OS randomness for a 0..15-byte padding length and bytes; explicit-padding encoding enforces 4-byte unpadded payload alignment and 0..15 padding; receive framing enforces fixed-U VersionD packet floor and strict `0x04000000` packet ceiling; VersionD quick-ACK encoding/decoding fails closed instead of borrowing generic intermediate semantics.
- Existing canonical shipping `native/telegram-network/src/lib.rs@54f509474391bb5118ba4678ba5c566ab3e74ec8` remains unchanged and continues routing `TcpTransport::send_payload/receive_payload` through `TransportFrameCodec::encode/decode`.
- Authored-only canonical contract commit is `cbc042f54b5b2bb7ec035ad68aaad5bd40263b4a`, contract blob `cd53d0b1b546aaceaf5ad46aaf6957dab5b32bf9`. Desktop provider contract mirror commit `551d641ba59ae6f2ef7e3c08bfbf884db8f7f2a3` has the same blob.
- Contracts cover explicit padding 0/15, unpadded alignment, fragmented header/body, 0..3 declared-body rejection, highest legal partial boundary, first illegal packet-size boundary, random shipping padding domain, VersionD quick-ACK rejection, and preservation of generic Intermediate quick-ACK behavior.
- No contract/build/typecheck/lint/schema/generator/integration/E2E/package/deployment/runtime validation was executed, and no GitHub Actions/status/check query or trigger was performed.

Connection-start nonce/AES-CTR prefix, Version1/VersionD secret handling, protocol/DC id obfuscation, quick-ACK lifecycle, broader connection scheduling/racing/retry, auth-key binding/persistence, arbitrary request history, gzip, complete difference decoding/application, broader source/legal closure, Stage-B executable evidence, and independent release ACCEPT remain outside this source-level responsibility.
