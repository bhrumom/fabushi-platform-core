# Telegram MTProto obfuscated connection-start wire

Responsibility: `TDRP-R9-MTPROTO-OBFUSCATED-CONNECTION-START-WIRE-001`

Stage: A source-level migration only. Validation remains `not-run`; acceptance remains false.

## Fixed source authority

- `telegramdesktop/tdesktop@f985723b552b30fe1c17d377d57f44e61a15a8c4`
- `Telegram/SourceFiles/mtproto/connection_tcp.cpp@d721a9058e1885d2964ded55bd4e4c7c2ce9e381`
- `Telegram/SourceFiles/mtproto/connection_tcp.h@ce09deab2f722ca22d371c851a7c43d939a9034b`
- `Telegram/SourceFiles/mtproto/mtproto_auth_key.h@31d0e3db369b309b8f2e9ea1d6b83b6a6d51b20c`
- `Telegram/SourceFiles/mtproto/mtproto_auth_key.cpp@cb4a4e5c3b94ed33d126068792536a08800615f2`
- `Telegram/SourceFiles/mtproto/details/mtproto_tcp_socket.cpp@b3130fd706b7cbfa33f7c9f54ac5e87abe9c3e10`
- Adjacent secret validation: `Telegram/SourceFiles/mtproto/mtproto_dc_options.cpp@d8499fc1df2ed7fdb88da687a04b824d243ff49a`
- Adjacent main/test protocol-DC-id derivation: `Telegram/SourceFiles/mtproto/session_private.cpp@13cbc76ec723386027b5b4dcac79999b791c4614`

Primary symbols: `TcpConnection::Protocol::Create`, `Version0::prepareKey`, `Version1::prepareKey`, `VersionD::id`, `TcpConnection::prepareConnectionStartPrefix`, `TcpSocket::isGoodStartNonce`, `aesCtrEncrypt`.

## Exact isolated wire contract

This responsibility owns direct-TCP connection-start obfuscation and the resulting stateful AES-CTR byte transform. It is separate from frame encoding, socket scheduling, request history, and auth-key lifecycle.

1. A direct connection starts from 64 random nonce bytes. Before protocol/DC fields are written, the nonce must be regenerated while any fixed-U direct-TCP reserved pattern is present:
   - first byte is `0xEF`;
   - first LE32 is ASCII `HEAD`, `POST`, `GET `, `0xEEEEEEEE`, `0xDDDDDDDD`, or `0x02010316`;
   - second LE32 is zero.
2. Accepted protocol-secret shapes are exact:
   - empty: Version0, abridged transport, protocol id `0xEFEFEFEF`;
   - exactly 16 bytes: Version1, abridged transport, protocol id `0xEFEFEFEF`;
   - exactly 17 bytes beginning `0xDD`: VersionD, padded-intermediate transport, protocol id `0xDDDDDDDD`;
   - at least 21 bytes beginning `0xEE`: VersionD, padded-intermediate transport, protocol id `0xDDDDDDDD`.
   Any other shape fails closed. VersionD uses only bytes 1..17 as its 16-byte transport secret; any `0xEE` suffix is outside this key derivation.
3. The outbound key source is nonce bytes 8..40 and outbound IV is nonce bytes 40..56.
4. Version0 copies the 32-byte key source directly. Version1/VersionD derive the 32-byte key as `SHA256(key_source || secret16)`.
5. The inbound key/IV source is a byte-reversed copy of nonce bytes 8..56. Its first 32 bytes are the key source and its last 16 bytes are the IV; Version1/VersionD apply the same SHA-256 secret derivation to that reversed key source.
6. Before the prefix transform, protocol id is written little-endian to nonce bytes 56..60 and the caller-supplied signed 16-bit protocol DC id is written little-endian to bytes 60..62. Bytes 62..64 remain from the nonce.
7. AES-256 CTR is byte-stream compatible with fixed-U OpenSSL `CRYPTO_ctr128_encrypt`: a 16-byte counter, big-endian counter increment after each generated block, and byte offset retained across calls.
8. The client send CTR processes all 64 modified nonce bytes. The transmitted 64-byte prefix is plaintext nonce bytes 0..56 followed by ciphertext bytes 56..64. The send CTR remains advanced by all 64 bytes for the first framed payload.
9. The receive CTR is initialized from the reversed key/IV and is not advanced by the client prefix; it decrypts server stream bytes from offset zero.
10. Connection-start framing mode is selected by the secret shape: Version0/Version1 use the existing abridged frame codec and VersionD uses the existing padded-intermediate frame codec. No standalone `0xEF` or `0xDDDDDDDD` transport header is emitted in addition to the obfuscated prefix.
11. The currently shipping Main-DC path may derive the raw protocol DC id from the canonical positive DC id, adding the fixed-U test-mode shift 10000 when `DcDirectory::test_mode()` is true. Media-cluster negative IDs, temporary-ID normalization, proxy/TLS variants, and broader endpoint scheduling remain separate responsibilities.

## Existing-owner disposition

Canonical wire owner remains `native/telegram-protocol/src/transport.rs` because it already owns MTProto TCP frame modes and can carry the pure connection-start/CTR state without creating a second runtime.

Canonical shipping carrier remains `native/telegram-network/src/lib.rs::TcpTransport`. The network owner may only:
- request a connection-start state from `telegram-protocol`;
- write the generated prefix once;
- encrypt framed outbound bytes and decrypt raw inbound bytes with that state;
- choose the existing frame codec mode returned by the protocol owner.

No new Telegram runtime/provider/socket scheduler is introduced.

## Explicit exclusions

This responsibility does not close:
- proxy/Web/TLS camouflage or TLS ClientHello/TlsBlock behavior;
- connection racing, retry, timeout, reconnect, endpoint ordering, or cancellation;
- media-cluster negative protocol DC ids or temporary-DC normalization;
- quick-ACK lifecycle/correlation;
- auth-key binding/persistence/expiry/renewal;
- arbitrary request histories;
- gzip;
- complete difference/differenceSlice decoding/application;
- already-closed plaintext/encrypted envelopes, secure inner messages, abridged framing, padded-intermediate framing, req_DH_params, or other handshake responsibilities.

## Authored-only contract requirements

Contracts must be written but not executed in Stage A and cover:
- exact accepted/rejected secret shapes and Version1/VersionD mode selection;
- `0xDD` and `0xEE` secrets with the same 16 key bytes deriving identical VersionD key material despite `0xEE` suffix bytes;
- every fixed-U direct nonce reserved-pattern rejection;
- first 56 prefix bytes remaining plaintext while bytes 56..64 are the CTR-transformed protocol/DC tail;
- protocol id and signed 16-bit DC id being inserted before encryption;
- Version0 direct-key versus Version1 SHA-256-derived-key divergence for the same nonce;
- stateful outbound CTR continuity after the 64-byte prefix;
- receive-direction key/IV reversal remaining distinct from outbound state;
- raw/non-obfuscated `TcpTransport::from_stream` behavior remaining available for focused transport fixtures, while the shipping Main-DC connection path uses obfuscated connection start.

No build, typecheck, lint, generator, schema validator, test, package, deployment, installation, runtime acceptance, Actions/status/check query, or CI trigger is permitted in this Stage-A slice.
