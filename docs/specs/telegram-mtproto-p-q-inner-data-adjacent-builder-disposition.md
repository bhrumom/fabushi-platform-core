# Telegram MTProto p_q_inner_data adjacent builder disposition

Status: Stage A source-level contract; executable validation deferred.
Fixed upstream: `telegramdesktop/tdesktop@f985723b552b30fe1c17d377d57f44e61a15a8c4`.

## Source authority

- `Telegram/SourceFiles/mtproto/scheme/mtproto.tl@76225965bfc794388db241f3f760db8ab79e84f8` defines:
  - `p_q_inner_data#83c95aec pq:string p:string q:string nonce:int128 server_nonce:int128 new_nonce:int256 = P_Q_inner_data`;
  - `p_q_inner_data_dc#a9f55f95 ... new_nonce:int256 dc:int = P_Q_inner_data`;
  - `p_q_inner_data_temp#3c6a84d4 ... new_nonce:int256 expires_in:int = P_Q_inner_data`;
  - `p_q_inner_data_temp_dc#56fddf88 ... new_nonce:int256 dc:int expires_in:int = P_Q_inner_data`.
- `Telegram/SourceFiles/mtproto/details/mtproto_dc_key_creator.cpp@13c57454f14b814b5923ea253048fb881ad8cf03` is the fixed-U shipping auth-key creation path. In `DcKeyCreator::pqAnswered`, persistent attempts serialize `MTP_p_q_inner_data_dc(..., protocolDcId)`; temporary attempts serialize `MTP_p_q_inner_data_temp_dc(..., protocolDcId, expiresIn)`.
- Fixed-U `DcKeyCreator` therefore has shipping reachability for the two `*_dc` variants. The no-`dc` variants `p_q_inner_data#83c95aec` and `p_q_inner_data_temp#3c6a84d4` are schema-adjacent but have no independent shipping call site in this auth-key path.

## Existing owner

The canonical source-neutral owner is the existing `native/telegram-protocol/src/handshake.rs` `AuthKeyHandshake`/TL builder boundary. It already exports `build_p_q_inner_data_dc` and `build_p_q_inner_data_temp_dc`; the persistent request-preparation path selects `build_p_q_inner_data_dc`, and the temporary request-preparation path selects `build_p_q_inner_data_temp_dc`.

This review must not introduce a second handshake owner, a dead serializer for a schema-only adjacent variant, or broaden into auth-key binding/persistence, network scheduling, request-history, gzip, or updates-difference lifecycle.

## Requirements

1. Revalidate `p_q_inner_data_dc#a9f55f95` as an exact persistent inner-object builder in the existing handshake owner: constructor, canonical TL bytes framing for pq/p/q, raw nonce/server_nonce/new_nonce preservation, and little-endian TL `int` dc field.
2. Revalidate shipping reachability: fixed-U persistent `DcKeyCreator::pqAnswered` selects the `*_dc` form, and canonical `AuthKeyHandshake::prepare_req_dh_params*` selects `build_p_q_inner_data_dc` before the existing RSA_PAD and `req_DH_params` framing.
3. Do not add a production delta when the existing builder already satisfies the exact responsibility.
4. Record `p_q_inner_data#83c95aec` as no-independent-shipping-reachability for the fixed-U `DcKeyCreator` path. Do not create a standalone builder merely to mirror an unused schema constructor.
5. Record `p_q_inner_data_temp#3c6a84d4` the same way. Its shipping temporary-key sibling is `p_q_inner_data_temp_dc#56fddf88`, already owned and separately closed by the existing temporary request-preparation slice.
6. Keep the existing `p_q_inner_data_temp_dc` implementation and temporary request preparation unchanged; this review neither reopens nor expands expiry/binding/persistence semantics.
7. Author but do not execute a focused contract that locks the exact `p_q_inner_data_dc` layout and factor-mismatch failure. Existing broader handshake contracts remain authoritative and unexecuted in Stage A.
8. Mirror the authored contract into the existing desktop tracked provider test copy; do not add a second desktop handshake implementation.
9. Update the unique desktop parity ledger, P0/source-closure and round evidence with `implemented / validation_status=not-run / accepted=false` for this isolated builder/disposition slice.
10. Preserve Stage A zero-CI: no Actions/status/check query or trigger, build, typecheck, lint, schema validator, generator, unit/contract/integration/E2E execution, package, deployment, installation or runtime acceptance.

## Failure / fail-closed boundaries

- Factor mismatch continues to fail with `HandshakeError::PqFactorsDoNotMatch`.
- A no-`dc` schema constructor without a fixed-U shipping caller is not treated as an implemented production responsibility merely because a serializer could be written.
- Permanent/temporary auth-key binding, lifetime enforcement, persistence/restore, connection scheduling, plaintext send lifecycle, arbitrary request history, gzip and complete difference decoding/application remain outside this slice.
- Stage B executable verification and independent release acceptance remain outstanding.

## Stage A acceptance state

This slice may advance only to source-level `implemented`, `validation_status=not-run`, `accepted=false`. Source review can establish no-production-delta and no-independent-shipping-reachability dispositions; it is not executable verification.


## Round 99 source-level compliance readback

Desktop bookkeeping checkpoint is `798965239cb0a72176de3106e4878809b4548bca`. Unique responsibility `TDRP-R9-MTPROTO-P-Q-INNER-DATA-ADJACENT-WIRE-001` is recorded as `implemented / coverage_disposition=rewritten / validation_status=not-run / accepted=false`.

| Requirement | Source-level status | Evidence / reason |
| --- | --- | --- |
| R1 | passed | Canonical `handshake.rs@87616f4b96a1775455da42b0bbed77711fb3c9f6` has `P_Q_INNER_DATA_DC_CONSTRUCTOR=0xa9f55f95` and `build_p_q_inner_data_dc` writing constructor, pq/p/q TL bytes, nonce, server_nonce, new_nonce and signed dc_id in exact order. |
| R2 | passed | Fixed-U `mtproto_dc_key_creator.cpp@13c57454f14b814b5923ea253048fb881ad8cf03` persistent branch selects `MTP_p_q_inner_data_dc(..., protocolDcId)`; canonical `AuthKeyHandshake::prepare_req_dh_params*` selects `build_p_q_inner_data_dc` before existing RSA_PAD/`req_DH_params`. |
| R3 | passed | No production source changed in this round; canonical handshake/lib production blobs remain `87616f4b96a1775455da42b0bbed77711fb3c9f6` / `abab39e42f100972484191f0e52e9edd9b88c6cf`. |
| R4 | passed | Fixed-U shipping auth-key creation always carries `protocolDcId`; no independent call site was found for `p_q_inner_data#83c95aec`, so no dead standalone builder was added. |
| R5 | passed | Same disposition for `p_q_inner_data_temp#3c6a84d4`; fixed-U temporary branch uses the already-closed `p_q_inner_data_temp_dc#56fddf88`. |
| R6 | passed | Existing temporary `*_temp_dc` implementation/request-preparation remains unchanged; binding/persistence/expiry semantics were not reopened. |
| R7 | authored / not run | Focused canonical contract commit `88cee9084231b22c3d575726c39634884a45f270` / blob `9c5ce9630c3865f19869b372d1f8f88d2d665487` covers exact layout/signed dc_id/EOF and factor mismatch. It was not executed. |
| R8 | passed | Desktop mirror commit `8c1866bf054281a4518d605ecbdd811300b1e8c4` has the identical contract blob; no desktop handshake implementation was introduced. |
| R9 | passed | Desktop bookkeeping commit `798965239cb0a72176de3106e4878809b4548bca` synchronizes the unique ledger, P0/source-closure and round99 evidence; coverage is `responsibilities_total=100, unknown=16172, unread=16145, omitted=0, unmapped_responsibilities=0`. |
| R10 | passed | This Stage-A slice performed source/blob readback and authored/saved docs/contracts only. No GitHub Actions/status/check query or trigger, build, typecheck, lint, schema validator, generator, unit/contract/integration/E2E execution, package, deploy, install or runtime acceptance was performed. |

Source-level compliance is complete for this isolated builder/disposition slice. This is not executable verification, candidate readiness, release acceptance, or closure of the broader migration.
