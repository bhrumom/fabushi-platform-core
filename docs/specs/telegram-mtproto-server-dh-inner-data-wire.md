# Telegram MTProto server_DH_inner_data wire contract

Status: active  
Owner: shared telegram-protocol / AuthKeyHandshake  
Last updated: 2026-11-01  
Related issue/task/PR: Fabushi migration task 7457eb1c-11a3-4626-8563-a3e495872536 round 49

## 1. Context / problem

Fixed upstream U defines server_DH_inner_data#b5890dba with fields nonce:int128, server_nonce:int128,
g:int, dh_prime:string, g_a:string, server_time:int. The shipping DcKeyCreator::dhParamsAnswered path decrypts
server_DH_params_ok.encrypted_answer with the temporary AES key/IV, parses this exact object, authenticates the
serialized object with the leading SHA-1 digest, validates the active nonce pair, applies DH safety validation,
and then continues to client_DH_inner_data.

At the live Fabushi checkpoint, the canonical Rust AuthKeyHandshake::receive_server_dh already delegates the OK
variant to decrypt_server_dh_inner_data. Source review shows that implementation already performs the fixed-U
wire/decryption checks required by this slice: exact constructor and field order, active nonce/server_nonce binding,
AES-IGE framing, SHA-1 authentication over the parsed TL object, residual alignment-padding bounding, signed
server_time decoding, and DH parameter validation. No production behavior delta is required. The Stage-A gap is
durable specification, exhaustive authored-only negative contracts, and migration bookkeeping.

## 2. Goal

Close the fixed-U server_DH_inner_data wire responsibility in the existing AuthKeyHandshake owner without adding a
second handshake/runtime/provider or expanding into auth-key persistence, temporary/permanent binding, connection
scheduling, multi-request history, gzip, or updates difference application.

## 3. Non-goals / out of scope

- No second Telegram handshake, runtime, provider, account, auth-key store, transport, or session owner.
- No temporary/permanent auth-key binding, expiry/renewal, persistence, destruction, teardown, or restart policy.
- No connection probing/racing, plaintext send scheduling, message-id generation, timeout, or transport retry policy.
- No arbitrary sent/resend/container history.
- No gzip_packed decompression or complete updates difference/differenceSlice decoder/application.
- No global/server-clock policy change. This slice parses and preserves server_time exactly; broader clock ownership is separate.
- No Stage-A build, typecheck, lint, schema validation, generator execution, test execution, Actions/status/check query,
  packaging, deployment, installation, or runtime acceptance.

## 4. Requirements

- R1: Preserve SERVER_DH_INNER_DATA_CONSTRUCTOR = 0xb5890dba.
- R2: decrypt_server_dh_inner_data SHALL reject an empty encrypted_answer and any encrypted_answer whose length is
  not a non-zero multiple of 16 bytes.
- R3: The temporary AES-IGE key/IV SHALL continue to be derived from the active new_nonce and server_nonce using the
  existing MTProto SHA-1 derivation, and decrypted key/IV material SHALL remain local to this handshake operation.
- R4: Decrypted plaintext SHALL be treated as 20 bytes of expected SHA-1 followed by exactly one serialized
  server_DH_inner_data object plus only AES block-alignment padding. Structurally impossible short plaintext SHALL fail closed.
- R5: The inner TL parser SHALL read constructor first and reject any constructor other than 0xb5890dba as
  UnexpectedServerDhInnerConstructor.
- R6: After the constructor, the parser SHALL decode exactly nonce:int128, server_nonce:int128, g:int,
  dh_prime:string, g_a:string, and server_time:int in fixed-U order.
- R7: The decoded nonce and server_nonce SHALL equal the active handshake pair before the object can be accepted.
- R8: The SHA-1 comparison SHALL authenticate the exact serialized TL object consumed by the parser, including TL
  bytes framing/padding, but excluding the residual AES alignment padding. Comparison SHALL remain constant-time.
- R9: Residual bytes after the parsed TL object SHALL be accepted only as AES alignment padding when their count is
  0..15. A residual length above 15 SHALL fail as InvalidServerDhPadding. This slice SHALL NOT reinterpret such
  padding as a second TL object.
- R10: g SHALL remain in the supported range, dh_prime SHALL be exactly 256 bytes and equal the audited 2048-bit
  safe prime, g_a SHALL be non-empty and at most 256 bytes, and the existing generator/public-value safety checks
  SHALL run before client DH preparation.
- R11: server_time SHALL be decoded as the fixed-U signed int field and preserved in ServerDhInnerData for the
  existing handshake continuation.
- R12: AuthKeyHandshake::receive_server_dh SHALL remain the live state owner. It SHALL only store server_dh,
  prepared_client_dh and advance AwaitingServerDh -> AwaitingDhGen after outer Server_DH_Params validation,
  inner decryption/authentication/validation, and client-DH preparation all succeed.
- R13: Exact positive/negative contracts SHALL be authored in the existing canonical protocol contract harness and
  mirrored into the existing desktop tracked provider contract surface. They SHALL NOT be executed in Stage A.

## 5. Verified current state

At platform-core checkpoint 54f1a26231827ac11a1ea0544dc4d1f7c3762c6c:

- native/telegram-protocol/src/handshake.rs@87616f4b96a1775455da42b0bbed77711fb3c9f6 already implements R1-R12.
- native/telegram-protocol/src/tl.rs@89256ab35184b6dc06a095ab34abb9f44e95a264 supplies bounded TL bytes parsing.
- native/telegram-protocol/tests/protocol_contract.rs@44d2a5d141e7d7854529379aaaad660853c8cbe3 has one valid
  server-DH decrypt/hash test and one SHA-1 corruption assertion, but does not yet cover the full constructor,
  truncation, nonce pair, malformed ciphertext, padding, parameter-length and receiver-state failure matrix.
- The desktop tracked protocol facade remains a consumer of the canonical shared implementation rather than a copied handshake core.

## 6. Target state

Production behavior remains byte-for-byte unchanged in handshake.rs. The repositories additionally contain authored-only
contracts proving the source-level failure matrix: valid exact object, malformed ciphertext lengths, unknown constructor,
all truncation cuts, nonce mismatch, server_nonce mismatch, SHA-1 mismatch, more-than-15 residual bytes, invalid g,
invalid dh_prime shape/trust, empty/oversized/unsafe g_a, exact server_time preservation, and handshake-state
non-mutation on rejected inner data.

## 7. Architecture and ownership boundaries

- native/telegram-protocol/src/handshake.rs: canonical decrypt/parser/DH validation and AuthKeyHandshake state owner.
- native/telegram-protocol/src/tl.rs: existing bounded TL primitive owner.
- native/telegram-protocol/tests/protocol_contract.rs: authored-only Stage-A contracts.
- Desktop tracked telegram-protocol facade/contracts: mirrored consumer contract only; no desktop handshake.rs.
- telegram-network/runtime/storage: unchanged.

No new owner or ADR is required because the existing owner is exact.

## 8. Interfaces / contracts / data flow

server_DH_params_ok.encrypted_answer
-> derive_tmp_aes_key_iv(active new_nonce, active server_nonce)
-> AES-IGE decrypt
-> [sha1(20) | serialized server_DH_inner_data | alignment padding 0..15]
-> constructor-first bounded TL decode
-> active nonce-pair validation
-> SHA-1 authentication of exactly consumed TL object
-> DH parameter safety validation
-> ServerDhInnerData
-> existing prepare_client_dh
-> existing AwaitingDhGen transition.

## 9. Constraints and non-functional requirements

- Upstream U remains telegramdesktop/tdesktop@f985723b552b30fe1c17d377d57f44e61a15a8c4.
- Schema blob remains Telegram/SourceFiles/mtproto/scheme/mtproto.tl@76225965bfc794388db241f3f760db8ab79e84f8.
- Shipping source anchor remains Telegram/SourceFiles/mtproto/details/mtproto_dc_key_creator.cpp@13c57454f14b814b5923ea253048fb881ad8cf03.
- Stage A may advance only to implemented / validation_status=not-run / accepted=false.
- Cryptographic primitives remain the existing audited dependencies; this slice does not invent cryptography.

## 10. Failure modes and edge cases

- Empty or non-block-aligned encrypted_answer is accepted.
- Decrypted plaintext is too short to contain digest plus fixed fields.
- Unknown inner constructor is consumed as fields.
- Any fixed field is truncated but accepted.
- nonce or server_nonce does not match the active handshake.
- SHA-1 is computed over residual AES padding, or padding is accidentally authenticated as TL content.
- More than 15 residual bytes are accepted as alignment padding.
- Invalid g, dh_prime, or g_a reaches client DH preparation.
- server_time is skipped, width-misread, or confused with padding.
- A rejected response advances AuthKeyHandshake state or stores partially validated DH state.
- Desktop gains a duplicate handshake implementation.
- This slice expands into binding/persistence/scheduler/history/gzip/difference ownership.

## 11. Implementation strategy

1. Commit this durable spec before contract changes.
2. Preserve the existing production handshake implementation because source review shows no production delta is needed.
3. Add deterministic helper-based authored-only contract cases around decrypt_server_dh_inner_data and the existing receiver.
4. Mirror the canonical contract file into the desktop tracked provider contract surface.
5. Add one responsibility row to the existing parity ledger and synchronize the existing P0/source-closure/evidence surfaces.
6. Perform remote source/blob readback only; run nothing in Stage A.
7. Finish with a source-level spec-compliance record that distinguishes authored evidence from unrun Stage-B validation.

## 12. Verification / test strategy

Stage A: write and manually inspect the contracts; do not execute them.

Stage B, only after full migration implementation closure: run the real build/unit/contract/integration/packaged campaign against
the frozen candidate and bind executable evidence to the exact candidate SHA.

Authored Stage-A cases:
- exact valid object with non-zero alignment padding and exact server_time;
- empty/non-multiple-of-16 ciphertext;
- unknown constructor;
- all truncation cuts of the canonical serialized inner object;
- active nonce and server_nonce mismatch;
- SHA-1 mismatch;
- residual alignment padding above 15;
- invalid generator;
- wrong dh_prime length and untrusted 256-byte prime;
- empty, oversized and numerically unsafe g_a;
- receiver failure leaves AwaitingServerDh and does not publish partial DH state.

## 13. Acceptance criteria / Definition of Done

- AC-1: Existing handshake.rs remains the only canonical production owner and requires no behavior delta.
- AC-2: Authored contracts cover R1-R12 positive/negative boundaries without being executed.
- AC-3: Desktop mirrors only the contract surface and keeps the canonical facade; no second handshake core is added.
- AC-4: The existing parity ledger/P0/source-closure/evidence surfaces record exact fixed-U and target blob/commit identities.
- AC-5: The new responsibility is implemented / validation_status=not-run / accepted=false.
- AC-6: No Stage-A executable validation or CI/status/check query is triggered.

## 14. Release / migration / rollback

Stage-A checkpoint branches only. No main integration or release is performed. Rollback is the immediately preceding checkpoint ref.
No data migration is introduced.

## 15. Observability / evidence

Only Git commit identities, file blob SHAs, fixed-U source identities and remote source readback are produced in Stage A.
No executable evidence is produced or claimed.

## 16. References / provenance

- telegramdesktop/tdesktop@f985723b552b30fe1c17d377d57f44e61a15a8c4:
  Telegram/SourceFiles/mtproto/scheme/mtproto.tl@76225965bfc794388db241f3f760db8ab79e84f8
- telegramdesktop/tdesktop@f985723b552b30fe1c17d377d57f44e61a15a8c4:
  Telegram/SourceFiles/mtproto/details/mtproto_dc_key_creator.cpp@13c57454f14b814b5923ea253048fb881ad8cf03
- Canonical AuthKeyHandshake::receive_server_dh and decrypt_server_dh_inner_data.

## 17. Spec compliance record

| Requirement / AC | Status | Evidence / reason |
| --- | --- | --- |
| R1-R12 | passed | Remote source readback confirms unchanged canonical `handshake.rs@87616f4b96a1775455da42b0bbed77711fb3c9f6` already performs non-empty AES-block framing, exact fixed-U constructor/field decode, active nonce binding, SHA-1 authentication of exactly consumed TL bytes, residual padding bound, signed server_time preservation and DH safety before the existing client-DH continuation. |
| R13 | passed | Canonical authored-only contracts are `protocol_contract.rs@256f421f54c41256646388833b634cf1191afd91` from commit `d724e29265e53a7ee7efd821c8bb37f6e4994cb2`; desktop sync `4323b97d3b6041731d5b547118fb746d2eba37a6` mirrors the same blob. They were not executed. |
| AC-1 | passed | Existing AuthKeyHandshake/decrypt_server_dh_inner_data remains the only canonical production owner; no production behavior change or second owner was added. |
| AC-2 | passed | Authored source covers exact signed server_time plus malformed ciphertext, unknown constructor, every serialized-inner truncation cut, nonce/server_nonce mismatch, SHA-1 mismatch, >15 residual padding, invalid generator, dh_prime length/trust, empty/oversized/unsafe g_a and rejected-inner state continuity. |
| AC-3 | passed | Desktop mirrors only the contract surface at blob `256f421f54c41256646388833b634cf1191afd91`; facade remains `abab39e42f100972484191f0e52e9edd9b88c6cf` and no desktop handshake core was introduced. |
| AC-4 | passed | Desktop bookkeeping commit `1ebaad62706f6d92f0ae537ea68f38e5883c9a1f` records the same fixed-U/source/target identities in the single ledger, both existing P0 surfaces, source-closure and Round 80 evidence. |
| AC-5 | passed | Ledger row `TDRP-R9-MTPROTO-SERVER-DH-INNER-DATA-WIRE-001` is `implemented / validation_status=not-run / accepted=false`; coverage is 90 responsibilities, 16172 unknown, 16145 unread, 0 omitted. |
| AC-6 | passed | This Stage-A slice used only Git/API source reads and writes; no build, lint, schema/generator execution, test execution, package/deploy/install/runtime acceptance, Actions trigger, or status/check query was performed. |
