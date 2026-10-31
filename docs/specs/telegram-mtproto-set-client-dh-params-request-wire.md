# Telegram MTProto set_client_DH_params request wire contract

Status: Stage A implementation contract; executable validation deferred.

## Fixed-U provenance

- Upstream: `telegramdesktop/tdesktop@f985723b552b30fe1c17d377d57f44e61a15a8c4`.
- `Telegram/SourceFiles/mtproto/scheme/mtproto.tl@76225965bfc794388db241f3f760db8ab79e84f8` defines `set_client_DH_params#f5045f1f nonce:int128 server_nonce:int128 encrypted_data:string = Set_client_DH_params_answer`.
- The same schema defines `req_DH_params#d712e4be nonce:int128 server_nonce:int128 p:string q:string public_key_fingerprint:long encrypted_data:string = Server_DH_Params`.
- `Telegram/SourceFiles/mtproto/details/mtproto_dc_key_creator.cpp@13c57454f14b814b5923ea253048fb881ad8cf03` is shipping-reachable: `pqAnswered` sends `MTPReq_DH_params(...)`; `dhClientParamsSend` advances to `Stage::WaitingDone` and sends `MTPSet_client_DH_params(attempt->data.nonce, attempt->data.server_nonce, MTP_string(...))`.
- `Telegram/SourceFiles/mtproto/session_private.cpp@13cbc76ec723386027b5b4dcac79999b791c4614` was reread as adjacent session context and contains no direct `MTPReq_DH_params` or `MTPSet_client_DH_params` construction.

## Current state

The canonical Rust `telegram-protocol` owner already has a public `build_req_dh_params` builder that serializes the fixed-U `req_DH_params#d712e4be` fields in schema order. Its existing contract reads the constructor, nonce, server_nonce, p, q, fingerprint, and encrypted_data and requires the reader to be finished, so the exact request-wire responsibility for `req_DH_params` is already source-level closed.

For `set_client_DH_params#f5045f1f`, `prepare_client_dh` currently writes the constructor, nonce, server_nonce, and encrypted_data inline after deriving/encrypting client DH material. The resulting body is covered only as part of the larger DH preparation contract; there is no independently callable pure request-body serializer. Exact set-client request-wire ownership is therefore still coupled to cryptographic/lifecycle work.

## Scope and ownership

This slice extracts only the exact `set_client_DH_params` request-body serialization into a pure canonical builder in the existing `telegram-protocol` request-wire owner, reuses it from the existing `prepare_client_dh` path, and synchronizes the tracked desktop provider copy. It does not change how client DH material, encryption, auth keys, salts, retry ids, or handshake state are produced.

It does not implement or broaden connection probing, transport racing, plaintext/not-secure packet framing, message-id generation, network send/retry/timeout, response correlation, auth-key persistence, session lifecycle, arbitrary sent/resend/container history, gzip, or updates difference handling.

## Requirements

- R1: Keep fixed-U constructor identity `SET_CLIENT_DH_PARAMS_CONSTRUCTOR = 0xf5045f1f`.
- R2: Export a pure `build_set_client_dh_params(nonce: &[u8; 16], server_nonce: &[u8; 16], encrypted_data: &[u8]) -> Result<Vec<u8>, TlError>` from the existing `telegram-protocol` facade.
- R3: The builder emits exactly the TL request body in fixed-U field order: little-endian constructor, raw nonce int128, raw server_nonce int128, and canonical TL string/bytes framing for encrypted_data, with no extra fields or trailing bytes.
- R4: `prepare_client_dh` may delegate only final request-body serialization to the pure builder; DH validation, random exponent generation, auth-key derivation, retry id, temporary AES encryption, server salt derivation, and return values remain unchanged.
- R5: The pure builder has no randomness, clock, network, persistence, scheduler, callback registry, request history, ACK/resend/container, gzip, or updates-difference side effects.
- R6: The already-existing `build_req_dh_params` exact serializer and its field/trailing contract remain unchanged; this slice must not duplicate or replace its handshake semantics.
- R7: Canonical and desktop tracked mirror files for the new request-wire API/export/contracts remain byte-identical after synchronization, without introducing a second desktop handshake lifecycle owner.
- R8: Dedicated exact-layout/string-framing and prepare-path-delegation contracts are authored in Stage A but no build, test, validator, GitHub Actions, status, or check execution/query is performed.

## Authored contracts

- exact little-endian `0xf5045f1f` constructor;
- patterned nonce and server_nonce bytes preserved unchanged;
- encrypted_data encoded with canonical TL string/bytes length and padding rules and no trailing bytes;
- `prepare_client_dh` request body equals `build_set_client_dh_params` over the encrypted_data it already produced;
- existing `build_req_dh_params` contract remains unchanged;
- public facade re-export and canonical/desktop changed-file byte identity.

## Acceptance

Source-level implementation may advance only to `implemented / validation_status=not-run / accepted=false` after remote readback confirms the builder, facade export, prepare-path delegation, authored contracts, desktop synchronization, and ledger/P0/source-closure evidence.

Executable verification is deferred to Stage B. No build, typecheck, lint, schema validator, generator, unit/contract/integration/E2E test, package, deployment, installation, GitHub Actions, checks, or statuses are executed or queried in Stage A.

## Fail-closed boundary

Connection-probe scheduling/transport racing, MTProto message-id generation, plaintext/not-secure packet framing, network send/retry/timeout, server response correlation/application, broader auth-key creation/persistence/session lifecycle, arbitrary multi-request history, gzip, complete difference/differenceSlice decoding/application, broader fixed-U source/legal closure, and independent release ACCEPT remain outside this slice.
