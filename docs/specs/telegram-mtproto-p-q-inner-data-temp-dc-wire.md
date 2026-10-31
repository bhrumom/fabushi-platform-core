# Telegram MTProto p_q_inner_data_temp_dc wire contract

Status: Stage A implementation contract; executable validation deferred.
Fixed upstream: `telegramdesktop/tdesktop@f985723b552b30fe1c17d377d57f44e61a15a8c4`.

## Source authority

- `Telegram/SourceFiles/mtproto/scheme/mtproto.tl@76225965bfc794388db241f3f760db8ab79e84f8` defines `p_q_inner_data_temp_dc#56fddf88 pq:string p:string q:string nonce:int128 server_nonce:int128 new_nonce:int256 dc:int expires_in:int = P_Q_inner_data`.
- `Telegram/SourceFiles/mtproto/details/mtproto_dc_key_creator.cpp@13c57454f14b814b5923ea253048fb881ad8cf03` stores the caller-provided temporary lifetime in `Attempt::expiresIn` when `pqSend` begins and, after `resPQ`, selects `MTP_p_q_inner_data_temp_dc(..., protocolDcId, expiresIn)` exactly when that value is non-zero.
- The same fixed-U file has no later consumer of `Attempt::expiresIn` after the inner object is built. The subsequent Req_DH_params, server_DH, client_DH and dh_gen stages use the same handshake state as the persistent path.
- The shipping Fabushi chain is `telegram-runtime::bootstrap_transport -> telegram-network::establish_auth_key* -> AuthKeyHandshake::prepare_req_dh_params*`.

## Existing owner

The canonical wire and handshake owner remains `native/telegram-protocol/src/handshake.rs`. The existing network owner remains `native/telegram-network/src/lib.rs`, and the existing runtime bootstrap remains `native/telegram-runtime/src/runtime.rs`. This slice must not introduce a second Telegram handshake, network stack, provider, auth-key store, scheduler or connection runtime.

The temporary lifetime is request-preparation input, not a new durable state owner. Fixed-U consumes it only while selecting and serializing the p_q inner object. Local auth-key expiration/persistence, permanent-key binding and broader PFS lifecycle remain explicitly outside this slice.

## Requirements

1. Add `P_Q_INNER_DATA_TEMP_DC_CONSTRUCTOR = 0x56fddf88` in the canonical handshake owner.
2. Add public pure `build_p_q_inner_data_temp_dc(response, factors, new_nonce, dc_id, expires_in)` that emits exactly the fixed-U constructor and fields in TL order, using the existing factor-match validation and canonical TL bytes/int encoders.
3. Add explicit temporary request preparation on `AuthKeyHandshake` with an `expires_in: i32` input. The temporary path must reject non-positive lifetimes and must reuse the existing Req_DH_params preparation state machine, trusted RSA key selection, new_nonce generation, RSA_PAD encryption and state transition rather than duplicating a second handshake.
4. Keep existing persistent `prepare_req_dh_params*` behavior byte-for-byte and semantically unchanged; it must continue using `p_q_inner_data_dc#a9f55f95`.
5. Treat `AuthKeyHandshake::prepare_temporary_req_dh_params*` as the production request-preparation wiring for this slice: it must consume the same `ResPqReceived` state and transition to the same `AwaitingServerDh` state as the persistent path, while selecting the temporary inner-object constructor. This is not a dead standalone serializer.
6. Keep `telegram-network::establish_auth_key` and runtime `bootstrap_transport` on the persistent path. The current runtime persists transport auth-key material without expiry/binding metadata, so routing a temporary key there would overclaim broader auth-key lifetime parity. Record that boundary fail-closed instead of silently persisting a temporary key as permanent material.
7. Re-export the new protocol builder/constructor through the canonical public facade and mirror the affected protocol facade/contract files into the desktop tracked provider copies without creating another state owner.
8. Author, but do not execute during Stage A, contracts for exact constructor/field order/TL padding/no trailing bytes, positive temporary request preparation, non-positive lifetime rejection before state mutation, and persistent-path non-regression.
9. Do not expand into connection scheduling/racing, message-id policy, plaintext transport framing, send/retry/timeout, arbitrary multi-request history, gzip, complete difference decoding, auth-key persistence/binding/renewal, packaging or release acceptance.

## Failure behavior

- Factor mismatch fails with the existing `PqFactorsDoNotMatch` error before serialization.
- A temporary preparation request with `expires_in <= 0` fails closed before mutating the handshake out of `ResPqReceived`.
- Invalid runtime `temporaryExpiresIn` values fail as an invalid request parameter rather than silently falling back to persistent mode.
- Existing persistent callers remain source-compatible and continue to select `p_q_inner_data_dc`.
- Network/runtime temporary-key establishment remains fail-closed until the existing persistence/session owner can represent expiry and binding semantics; no temporary key is written into the current permanent transport snapshot.

## Stage A acceptance state

Implementation may advance only to `implemented`, `validation_status=not-run`, `accepted=false`. Tests and validators are authored but not executed. No GitHub Actions/status/check query or trigger, build, typecheck, lint, schema validator, generator, unit/contract/integration/E2E, package, deployment, installation or runtime acceptance is permitted in Stage A.

## Fail-closed boundary

Local temporary-auth-key expiry enforcement, key renewal, permanent-key binding, broader auth-key persistence/session lifecycle, connection probing/racing, transport send/retry/timeout, arbitrary sent/resend/container histories, gzip, complete updates difference decoding/application, broader fixed-U source/legal closure, Stage-B executable evidence and independent release ACCEPT remain open.
