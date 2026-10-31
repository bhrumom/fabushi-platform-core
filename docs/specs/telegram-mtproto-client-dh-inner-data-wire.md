# Telegram MTProto client_DH_inner_data wire contract

Status: Stage A implementation contract; executable validation deferred.
Fixed upstream: `telegramdesktop/tdesktop@f985723b552b30fe1c17d377d57f44e61a15a8c4`.

## Source authority

- `Telegram/SourceFiles/mtproto/scheme/mtproto.tl@76225965bfc794388db241f3f760db8ab79e84f8` defines `client_DH_inner_data#6643b654 nonce:int128 server_nonce:int128 retry_id:long g_b:string = Client_DH_Inner_Data`.
- `Telegram/SourceFiles/mtproto/details/mtproto_dc_key_creator.cpp@13c57454f14b814b5923ea253048fb881ad8cf03` constructs `MTP_client_DH_inner_data` in the shipping DH client-params path before SHA-1 prefixing, random block padding, temporary AES-IGE encryption, and `MTPSet_client_DH_params` send.
- `Telegram/SourceFiles/mtproto/session_private.cpp@13cbc76ec723386027b5b4dcac79999b791c4614` was reread as adjacent session context and contains no direct `client_DH_inner_data` construction.

## Existing owner

The canonical owner remains `native/telegram-protocol/src/handshake.rs`. Its existing `prepare_client_dh` already owns server-DH validation, random exponent generation, `g_b` and auth-key derivation, retry-id input, SHA-1 prefixing, randomized AES-block padding, temporary AES-IGE encryption, server-salt derivation, and `PreparedClientDh`. This slice introduces no second handshake runtime or transport owner.

## Requirements

1. Keep `CLIENT_DH_INNER_DATA_CONSTRUCTOR = 0x6643b654` as the fixed-U constructor authority.
2. Add a public pure `build_client_dh_inner_data(nonce, server_nonce, retry_id, g_b)` serializer in the existing handshake owner.
3. Serialize exactly constructor, raw nonce, raw server_nonce, little-endian TL `long` retry_id, and canonical TL string/bytes framing for caller-owned `g_b`, with no extra fields.
4. Refactor only the existing inner-object framing in `prepare_client_dh` to delegate to the pure builder; all cryptographic validation, randomness, auth-key derivation, hash/padding/encryption, salt derivation, and final `set_client_DH_params` framing stay in their existing owners.
5. The pure builder must not own network send/retry/timeout, message-id generation, request correlation/history, persistence, scheduler state, gzip, updates difference handling, or connection/auth-key lifecycle.
6. Re-export the builder through the canonical public facade and mirror that facade/contract surface into the desktop tracked provider without copying a second handshake implementation.
7. Author exact constructor/field-order/TL-padding/no-trailing contracts in the existing protocol contract harness. Do not execute them during Stage A.
8. Keep adjacent `p_q_inner_data_temp_dc#56fddf88` fail-closed until the existing handshake lifecycle can explicitly represent temporary-key `expires_in`; a standalone unused serializer is not sufficient production closure.

## Stage A acceptance state

Implementation may advance only to `implemented`, `validation_status=not-run`, `accepted=false`. Executable build/test/validator/CI evidence is intentionally deferred to Stage B.

## Fail-closed boundary

Temporary-key `p_q_inner_data_temp_dc` lifecycle, connection probing/racing, plaintext/not-secure framing, message-id generation, network send/retry/timeout, server response correlation/application, broader auth-key persistence/session lifecycle, arbitrary sent/resend/container histories, gzip, complete difference decoding/application, broader fixed-U source/legal closure, and independent release ACCEPT remain outside this slice.
