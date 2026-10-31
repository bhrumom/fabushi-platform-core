# Telegram MTProto req_pq_multi request wire contract

Status: Stage A implementation contract; executable validation deferred.

## Fixed-U provenance

- Upstream: `telegramdesktop/tdesktop@f985723b552b30fe1c17d377d57f44e61a15a8c4`.
- `Telegram/SourceFiles/mtproto/scheme/mtproto.tl@76225965bfc794388db241f3f760db8ab79e84f8` defines `req_pq_multi#be7e8ef1 nonce:int128 = ResPQ`.
- `Telegram/SourceFiles/mtproto/details/mtproto_dc_key_creator.cpp@13c57454f14b814b5923ea253048fb881ad8cf03` is shipping-reachable: `DcKeyCreator::pqSend` sets `Stage::WaitingPQ`, generates the attempt nonce, and passes `MTPReq_pq_multi(attempt->data.nonce)` to `sendNotSecureRequest`.
- `Telegram/SourceFiles/mtproto/session_private.cpp@13cbc76ec723386027b5b4dcac79999b791c4614` was reread as adjacent session context and contains no `req_pq_multi` branch.

## Current state

The existing canonical Rust `telegram-protocol` owner already exposes `REQ_PQ_MULTI_CONSTRUCTOR = 0xbe7e8ef1` from `handshake.rs`, and `AuthKeyHandshake::begin` currently emits the same 20-byte request body inline before returning its plaintext envelope. There is no independently callable pure `req_pq_multi` request-body builder, so exact serialization is coupled to the handshake state transition.

## Scope and ownership

This slice moves only the exact request-body serialization into a pure canonical builder inside the existing `telegram-protocol` owner, reuses that builder from the existing `AuthKeyHandshake::begin` path without changing the path's state transition or envelope semantics, and synchronizes the byte-identical desktop provider copy.

It does not implement, broaden, or claim ownership of connection probing, endpoint/transport racing, nonce generation, message-id generation, plaintext/not-secure envelope encoding, network send/retry/timeout, `ResPQ` correlation/application, RSA/DH negotiation, auth-key persistence, session lifecycle, arbitrary sent/resend/container history, gzip, or updates difference handling.

## Requirements

- R1: Keep the canonical public `REQ_PQ_MULTI_CONSTRUCTOR = 0xbe7e8ef1` identity unchanged.
- R2: Export a pure `build_req_pq_multi(nonce: &[u8; 16]) -> Vec<u8>` from the existing `telegram-protocol` facade.
- R3: The builder emits exactly 20 bytes: four little-endian constructor bytes followed by the caller's 16 nonce bytes unchanged, with no TL bytes/string framing, padding, reversal, or reinterpretation.
- R4: `AuthKeyHandshake::begin` may delegate only request-body serialization to `build_req_pq_multi`; its existing invalid-state guard, caller-supplied message id, transition to `AwaitingResPq`, and `PlaintextEnvelope` behavior must remain unchanged.
- R5: The builder is synchronous and side-effect free: no randomness, clock, network, persistence, scheduler, callback registry, request history, ACK/resend/container, gzip, or updates-difference interaction.
- R6: Canonical and desktop provider source for the tracked mirrored request-wire files (`api.rs`, `lib.rs`, and `tests/protocol_contract.rs`) remain byte-identical after synchronization; desktop does not gain a second handshake lifecycle owner.
- R7: Dedicated exact-layout/raw-nonce contracts are authored in Stage A but no build, test, validator, Actions, status, or check execution/query is performed.

## Authored contracts

- exact little-endian `0xbe7e8ef1` constructor followed by the raw 16-byte nonce;
- all-zero, all-`0xff`, and patterned nonce identity without TL bytes framing;
- existing `AuthKeyHandshake::begin` body remains byte-equal to the pure builder while preserving the existing `AwaitingResPq` transition;
- public facade re-export;
- canonical/desktop changed-file byte identity.

## Acceptance

Source-level implementation may advance to `implemented / validation_status=not-run / accepted=false` only after remote readback confirms the builder, facade export, existing begin-path delegation, authored contracts, desktop synchronization, and ledger/P0/source-closure evidence.

Executable verification is explicitly deferred to Stage B. No build, typecheck, lint, schema validator, generator, unit/contract/integration/E2E test, package, deployment, installation, GitHub Actions, checks, or statuses are executed or queried in Stage A.

## Fail-closed boundary

Connection-probe scheduling/transport racing, attempt nonce generation, MTProto message-id generation, plaintext/not-secure packet framing, send/retry/timeout, response correlation, `ResPQ` application, the broader auth-key handshake lifecycle, arbitrary multi-request history, gzip, complete difference/differenceSlice decoding/application, broader source/legal closure, and independent release ACCEPT remain outside this slice.

## Spec compliance record

| Requirement | Status | Evidence / reason |
| --- | --- | --- |
| R1 | passed | Public `REQ_PQ_MULTI_CONSTRUCTOR=0xbe7e8ef1` is exported from canonical `api.rs@3e3b250b709ce2f307f896dda6d1c2da55559f7b`. |
| R2 | passed | Public `build_req_pq_multi(nonce:&[u8;16])` is present in canonical `api.rs` and re-exported by `lib.rs@2d48c2fe1c677b4b4672e7e76d8bd0a29a7ac003`. |
| R3 | passed | Source readback shows the builder writes the little-endian constructor and raw int128 bytes only; exact-layout/raw-nonce contracts are authored in `protocol_contract.rs@bf5aeb85f10baeeb7e645a660fd3bf26259e5989`. |
| R4 | passed | `AuthKeyHandshake::begin` in `handshake.rs@b1b22af0e83b577d6152beb11a75e44eb76ffdbe` keeps its existing invalid-state guard and `AwaitingResPq` transition and delegates only body construction to `build_req_pq_multi`. |
| R5 | passed | The builder is a synchronous TL writer over caller-owned nonce bytes and contains no randomness, clock, network, persistence, scheduler, callback, request-history, gzip, or updates interaction. |
| R6 | passed | Remote readback confirms canonical and desktop tracked mirrors are byte-identical: api `3e3b250b709ce2f307f896dda6d1c2da55559f7b`, lib `2d48c2fe1c677b4b4672e7e76d8bd0a29a7ac003`, contracts `bf5aeb85f10baeeb7e645a660fd3bf26259e5989`; no desktop handshake lifecycle file was introduced. |
| R7 | passed | Dedicated serializer/begin-body contracts are authored and no build/test/validator/Actions/status/check execution or query was performed in Stage A. |


## Round 72 source-level compliance readback

Spec-first commit `c5b3021a760892eaf05fcd9b40dc35dff4c2c783` established this contract before implementation. Canonical implementation commits `60e4b9165c0f41804bf68e7270aa52f39e824f14` and `591176a5ba091bb2a00f925d38ed87a1fdae0b11` produce api `3e3b250b709ce2f307f896dda6d1c2da55559f7b`, handshake `b1b22af0e83b577d6152beb11a75e44eb76ffdbe`, lib `2d48c2fe1c677b4b4672e7e76d8bd0a29a7ac003`, and authored-contract blob `bf5aeb85f10baeeb7e645a660fd3bf26259e5989`.

Desktop synchronization commit `59e08389bae4ab2cdcbb3f67808da44dff452ca9` mirrors api/lib/contracts byte-for-byte. Desktop bookkeeping commit `f62e64429136d5355fed4089cb6adbc5863a4eb3` records `TDRP-R9-MTPROTO-REQ-PQ-MULTI-REQUEST-WIRE-001` with `responsibilities_total=82`, `unknown=16172`, `unread=16145`, `omitted=0`, `implementation_status=implemented`, `validation_status=not-run`, and `accepted=false`.

R1-R7 are satisfied at source/ownership/bookkeeping level. This does not constitute executable verification or release acceptance. Connection probing/racing, nonce/message-id generation, plaintext/not-secure framing, network send/retry/timeout, response correlation/application, broader auth-key creation/persistence/session lifecycle, arbitrary sent/resend/container histories, gzip, complete difference/differenceSlice decoding/application, broader fixed-U source/legal closure, Stage-B evidence and independent release ACCEPT remain open/fail-closed.

No GitHub Actions/status/check query or trigger, build, typecheck, lint, schema validator, generator, unit/contract/integration/E2E test, package, deployment, installation, or runtime acceptance was performed.
