# Telegram MTProto req_DH_params request wire contract

Status: Stage A implementation contract; executable validation deferred.

## Fixed-U provenance

- Upstream: `telegramdesktop/tdesktop@f985723b552b30fe1c17d377d57f44e61a15a8c4`.
- `Telegram/SourceFiles/mtproto/scheme/mtproto.tl@76225965bfc794388db241f3f760db8ab79e84f8` defines `req_DH_params#d712e4be nonce:int128 server_nonce:int128 p:string q:string public_key_fingerprint:long encrypted_data:string = Server_DH_Params`.
- `Telegram/SourceFiles/mtproto/details/mtproto_dc_key_creator.cpp@13c57454f14b814b5923ea253048fb881ad8cf03` is shipping-reachable: `DcKeyCreator::pqAnswered` factors the active `resPQ.pq`, selects a trusted RSA fingerprint, encrypts the corresponding p_q_inner_data object, and sends `MTPReq_DH_params(nonce, server_nonce, p, q, fingerprint, encrypted_data)`.
- Existing canonical owner: `native/telegram-protocol/src/handshake.rs` `AuthKeyHandshake::prepare_req_dh_params*` and `build_req_dh_params`.

## Goal

Close `req_DH_params#d712e4be` as its own exact request-wire responsibility in the existing telegram-protocol handshake owner. Preserve current shipping wiring and fail closed if the caller supplies p/q factors that do not multiply back to the active `resPQ.pq`.

## Non-goals

This slice does not own or broaden RSA_PAD cryptography, key selection policy beyond the already-selected fingerprint check, temporary/permanent auth-key binding, expiry/renewal, persistence/restore, connection probing or racing, plaintext/not-secure packet framing, message-id generation, send/retry/timeout, response correlation, arbitrary request history, gzip, or updates difference decoding/application.

## Requirements

1. Keep fixed-U constructor identity `REQ_DH_PARAMS_CONSTRUCTOR = 0xd712e4be`.
2. The existing public `build_req_dh_params` remains the unique pure serializer and writes exactly: constructor, nonce, server_nonce, p, q, public_key_fingerprint, encrypted_data.
3. p and q are canonical unsigned big-endian factor bytes emitted through TL bytes/string framing.
4. The builder rejects a factor set unless `response.pq` decodes to `factors.pq` and `factors.p * factors.q == factors.pq`, using the existing `HandshakeError::PqFactorsDoNotMatch`.
5. The builder rejects a fingerprint not present in the active `resPQ.server_public_key_fingerprints` with the existing `NoTrustedServerKey`.
6. The builder rejects empty encrypted inner data with the existing `EmptyEncryptedInnerData`.
7. `AuthKeyHandshake::prepare_req_dh_params*` continues to factor the active response, select the trusted RSA key, build the already-closed p_q_inner_data_dc or p_q_inner_data_temp_dc, RSA_PAD-encrypt it, then call this builder before advancing to `AwaitingServerDh`.
8. No second handshake owner or dead duplicate serializer is introduced.
9. Focused Stage-A contracts are authored but not executed: exact field order/EOF, mismatched-factor rejection, untrusted-fingerprint rejection, and empty encrypted-data rejection.
10. Desktop synchronization updates only the existing tracked provider-facing contract/facade artifacts that exist in `bhrumom/fabushi-desktop`; no nonexistent duplicate desktop handshake implementation is created.
11. The unique desktop parity ledger, P0/source-closure and round evidence record `implemented / validation_status=not-run / accepted=false`.
12. Preserve Stage A zero-CI: no Actions/status/check query or trigger, build, typecheck, lint, schema validator, generator, unit/contract/integration/E2E execution, package, deployment, installation or runtime acceptance.

## Acceptance

Source-level acceptance requires remote readback of the canonical production change, authored-only contract, desktop tracked mirror, unique ledger/P0/source-closure/evidence updates, and final compliance note. Executable verification remains Stage B work; `accepted` stays false.

## Fail-closed boundary

Any factor mismatch, fingerprint mismatch, or empty encrypted inner payload is rejected before a request body is returned. Broader auth-key lifecycle, transport scheduling and arbitrary request-history semantics remain outside this responsibility.


## Round 100 source-level compliance readback

Desktop bookkeeping checkpoint is `cb082b48228fbb2ee43021f41592b605d072bfb9`. Unique responsibility `TDRP-R9-MTPROTO-REQ-DH-PARAMS-REQUEST-WIRE-001` is recorded as `implemented / coverage_disposition=rewritten / validation_status=not-run / accepted=false`.

| Requirement | Source-level status | Evidence / reason |
| --- | --- | --- |
| R1 | passed | Canonical `REQ_DH_PARAMS_CONSTRUCTOR=0xd712e4be` remains in `handshake.rs@b6572c91a450a240c56c15ca80585afb97c19839`. |
| R2 | passed | Existing public `build_req_dh_params` remains the unique pure serializer and writes constructor, nonce, server_nonce, p, q, fingerprint and encrypted_data in fixed-U order. |
| R3 | passed | Existing builder emits `FactoredPq::p_bytes/q_bytes` through canonical TL bytes framing; exact reader/EOF contract is authored in `protocol_contract.rs@071b72be6772117fb174daf86d5c9a19ce372b73`. |
| R4 | passed at source level | `build_req_dh_params` now re-decodes active `response.pq`, checks `factors.pq`, and checks `p*q==pq`; mismatch returns existing `PqFactorsDoNotMatch`. |
| R5 | passed | Existing active resPQ fingerprint membership check remains and authored contract records `NoTrustedServerKey`. |
| R6 | passed | Existing non-empty encrypted payload guard remains and authored contract records `EmptyEncryptedInnerData`. |
| R7 | passed | `AuthKeyHandshake::prepare_req_dh_params*` still factors active resPQ, selects trusted RSA key, builds/encrypts the existing *_dc inner variant, calls `build_req_dh_params`, then advances to `AwaitingServerDh`. |
| R8 | passed | No second handshake owner or duplicate serializer was introduced. |
| R9 | authored / not run | Focused exact-layout/EOF plus factor/fingerprint/empty-payload negative contracts are in canonical `071b72be6772117fb174daf86d5c9a19ce372b73`; no executable validation was run. |
| R10 | passed | Desktop has no tracked handshake.rs duplicate; existing contract mirror was synchronized byte-identically at `0e0188f5d1ebf96d2ee21444e1f7b65994c88ee2` / `071b72be6772117fb174daf86d5c9a19ce372b73`. |
| R11 | passed | Desktop bookkeeping `cb082b48228fbb2ee43021f41592b605d072bfb9` synchronizes unique ledger/P0/source-closure/round100 evidence with coverage `responsibilities_total=101, unknown=16172, unread=16145, omitted=0, unmapped_responsibilities=0`. |
| R12 | passed | No GitHub Actions/status/check query or trigger, build, typecheck, lint, schema validator, generator, unit/contract/integration/E2E execution, package, deployment, installation or runtime acceptance was performed. |

Source-level compliance is complete for this isolated request-wire slice. This is not executable verification, candidate readiness, release acceptance, or closure of the broader migration. Auth-key binding/persistence/expiry/renewal, connection scheduling/racing/send retry, arbitrary request histories, gzip, complete difference/differenceSlice decoding/application, broader source/legal closure, Stage-B evidence and independent release ACCEPT remain open.
