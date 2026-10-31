# Telegram MTProto destroy_auth_key result wire contract

Status: Stage A implementation contract; executable validation deferred.
Fixed upstream: `telegramdesktop/tdesktop@f985723b552b30fe1c17d377d57f44e61a15a8c4`.

## Source authority and adjacent constructor disposition

- `Telegram/SourceFiles/mtproto/scheme/mtproto.tl@76225965bfc794388db241f3f760db8ab79e84f8` defines:
  - `p_q_inner_data#83c95aec ... = P_Q_inner_data`;
  - `p_q_inner_data_temp#3c6a84d4 ... expires_in:int = P_Q_inner_data`;
  - `p_q_inner_data_dc#a9f55f95 ... dc:int = P_Q_inner_data`;
  - `p_q_inner_data_temp_dc#56fddf88 ... dc:int expires_in:int = P_Q_inner_data`;
  - `destroy_auth_key_ok#f660e1d4 = DestroyAuthKeyRes`;
  - `destroy_auth_key_none#0a9f2259 = DestroyAuthKeyRes`;
  - `destroy_auth_key_fail#ea109b13 = DestroyAuthKeyRes`.
- `Telegram/SourceFiles/mtproto/details/mtproto_dc_key_creator.cpp@13c57454f14b814b5923ea253048fb881ad8cf03` constructs `DcKeyCreator` with a required `protocolDcId`, selects `MTP_p_q_inner_data_dc` for persistent key creation and `MTP_p_q_inner_data_temp_dc` for temporary key creation, and has no shipping branch selecting the two no-`dc` variants. Therefore `p_q_inner_data#83c95aec` and `p_q_inner_data_temp#3c6a84d4` are schema-adjacent but do not form independent fixed-U shipping auth-key responsibilities in this path; no duplicate Fabushi builders are added for them.
- The adjacent `bind_auth_key_inner#75a3f765` responsibility depends on temporary/permanent key binding, expiry and session lifecycle state that the current persistence owner cannot yet represent. That responsibility remains fail-closed.
- `Telegram/SourceFiles/mtproto/mtp_instance.cpp@d25fe3b72298fab371854f80186ee8e4777ea087` has a shipping key-destroyer path that sends `MTPDestroy_auth_key()`, reads `MTPDestroyAuthKeyRes`, and distinguishes ok/fail/none.

## Existing owner

The exact response-wire responsibility belongs to the existing canonical `native/telegram-protocol/src/api.rs` owner and its public facade. The desktop tracked provider copies remain byte-identical mirrors. This slice does not create a second auth-key/session lifecycle owner.

## Requirements

1. Export the three fixed-U response constructors:
   - `DESTROY_AUTH_KEY_OK_CONSTRUCTOR = 0xf660e1d4`;
   - `DESTROY_AUTH_KEY_NONE_CONSTRUCTOR = 0x0a9f2259`;
   - `DESTROY_AUTH_KEY_FAIL_CONSTRUCTOR = 0xea109b13`.
2. Add a typed `DestroyAuthKeyResult` with `Ok`, `None`, and `Fail` variants.
3. Add `parse_destroy_auth_key_result(input)` that accepts exactly one of the three zero-field constructors and rejects truncation, unknown constructors, and trailing bytes.
4. Re-export the parser, result type, and constructors through the canonical public facade and mirror the affected protocol api/lib/contract files into the existing desktop provider path.
5. Author, but do not execute during Stage A, contracts covering all three variants plus truncation, unknown-constructor, and trailing-byte rejection.
6. Keep destruction eligibility, local key deletion, persisted-key mutation, transport/session teardown, reconnect, error retry, and broader temporary/permanent key lifecycle out of this slice. Those semantics remain fail-closed until an exact existing owner represents them.
7. Do not add builders for `p_q_inner_data#83c95aec` or `p_q_inner_data_temp#3c6a84d4` because the fixed-U shipping `DcKeyCreator` path always carries `protocolDcId` and selects the existing `*_dc` variants.
8. Do not expand into connection scheduling/racing, arbitrary multi-request sent/resend/container history, gzip, complete updates difference decoding/application, packaging, deployment or release acceptance.

## Failure behavior

- Inputs shorter than one TL constructor fail through the existing TL reader error.
- Any constructor other than the three `DestroyAuthKeyRes` variants fails closed with a typed unexpected-constructor error.
- Any bytes after the zero-field constructor fail with the existing trailing-bytes error.
- Parsing has no auth-key deletion, persistence, transport, session, retry or network side effect.

## Stage A acceptance state

Implementation may advance only to `implemented`, `validation_status=not-run`, `accepted=false`. Contracts are authored but not executed. No GitHub Actions/status/check query or trigger, build, typecheck, lint, schema validator, generator, unit/contract/integration/E2E, package, deployment, installation or runtime acceptance is permitted in Stage A.
