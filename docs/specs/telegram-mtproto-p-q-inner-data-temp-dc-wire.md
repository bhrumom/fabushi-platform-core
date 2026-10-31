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
- Network/runtime temporary establishment is not exposed by this slice; any future temporary-lifetime input must remain unavailable or fail closed until the existing persistence/session owner represents expiry and binding semantics.
- Existing persistent callers remain source-compatible and continue to select `p_q_inner_data_dc`.
- Network/runtime temporary-key establishment remains fail-closed until the existing persistence/session owner can represent expiry and binding semantics; no temporary key is written into the current permanent transport snapshot.

## Stage A acceptance state

Implementation may advance only to `implemented`, `validation_status=not-run`, `accepted=false`. Tests and validators are authored but not executed. No GitHub Actions/status/check query or trigger, build, typecheck, lint, schema validator, generator, unit/contract/integration/E2E, package, deployment, installation or runtime acceptance is permitted in Stage A.

## Fail-closed boundary

Local temporary-auth-key expiry enforcement, key renewal, permanent-key binding, broader auth-key persistence/session lifecycle, connection probing/racing, transport send/retry/timeout, arbitrary sent/resend/container histories, gzip, complete updates difference decoding/application, broader fixed-U source/legal closure, Stage-B executable evidence and independent release ACCEPT remain open.


## Round 75 source-level compliance readback

Canonical implementation readback at platform-core head `bcd516892fa49407a4de757e05235ed585ad4338` confirms handshake blob `3e740e0714dfca71bb908ec46f182e98605582e5`, facade blob `a2dc3f48a15c7a14657f598d7a5ce4b1fed9ed7f`, and authored-contract blob `748ca5f7962ff0704a630144c7368c067d212de3`. Desktop tracked facade/contracts at `8206a6cc5bdea79e197dc70fd800e021480f3c7c` are byte-identical to the canonical blobs.

| Requirement | Source-level status | Evidence / reason |
| --- | --- | --- |
| R1 | passed | `P_Q_INNER_DATA_TEMP_DC_CONSTRUCTOR = 0x56fddf88` is present in canonical `handshake.rs`. |
| R2 | passed | Public pure `build_p_q_inner_data_temp_dc` writes pq/p/q, nonce, server_nonce, new_nonce, dc_id and expires_in in fixed-U TL order after the existing pq-factor match check. |
| R3 | passed | `prepare_temporary_req_dh_params*` rejects `expires_in <= 0` before entering the shared preparation helper and otherwise reuses factorization, trusted RSA selection, new_nonce generation, RSA_PAD, req_DH_params framing and the existing state transition. |
| R4 | passed | Existing `prepare_req_dh_params*` delegates to the same helper with `None`; source readback shows the persistent branch still selects `build_p_q_inner_data_dc`. |
| R5 | passed | Temporary preparation consumes the existing `ResPqReceived` state and transitions to `AwaitingServerDh`; there is no standalone second handshake owner. |
| R6 | passed / fail-closed boundary preserved | telegram-network `establish_auth_key` and runtime `bootstrap_transport` were intentionally not changed to temporary establishment because current persisted transport state has no expiry/binding metadata. No temporary key is silently persisted as permanent material. |
| R7 | passed | Canonical facade exports the builder/constructor; desktop tracked facade/contracts are byte-identical and no second implementation was added. |
| R8 | authored / not run | Contracts cover exact field order, expires_in=86400, no trailing bytes, non-positive lifetime rejection before state mutation, and temporary prepare transition. Stage A executable validation remains intentionally absent. |
| R9 | passed | No connection scheduling/racing, message-id policy, plaintext framing, send/retry/timeout, multi-request history, gzip, complete difference decoder, auth-key persistence/binding/renewal, packaging or release logic was added. |

This is source/ownership/bookkeeping compliance, not executable verification or release acceptance. Ledger responsibility `TDRP-R9-MTPROTO-P-Q-INNER-DATA-TEMP-DC-WIRE-001` is `implemented / validation_status=not-run / accepted=false`; desktop ledger blob is `971489ca6ff27838b1ef884a24ced884f60da4ea` with `responsibilities_total=85`, `unknown=16172`, `unread=16145`, `omitted=0`.

Broader temporary-key expiry enforcement, permanent-key binding, renewal/persistence, network/runtime temporary establishment, connection scheduling/racing, plaintext/send lifecycle, arbitrary sent/resend/container histories, gzip, complete difference decoding/application, broader fixed-U source/legal closure, Stage-B evidence and independent release ACCEPT remain open/fail-closed.

No GitHub Actions/status/check query or trigger, build, typecheck, lint, schema validator, generator, unit/contract/integration/E2E test, package, deployment, installation or runtime acceptance was performed.


## Round 75 wording correction

Final readback removed the stale pre-boundary statement that implied runtime `temporaryExpiresIn` input already existed. The implemented contract is unambiguous: this slice ends at the existing `AuthKeyHandshake::prepare_temporary_req_dh_params*` request-preparation lifecycle; telegram-network/runtime temporary establishment is not exposed and remains fail-closed pending expiry/binding-aware persistence/session ownership. No production code changed in this correction and no executable validation was performed.
