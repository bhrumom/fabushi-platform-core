# Telegram MTProto dh_gen result wire contract

Status: active  
Owner: shared telegram-protocol / AuthKeyHandshake  
Last updated: 2026-11-01  
Related issue/task/PR: Fabushi migration task 7457eb1c-11a3-4626-8563-a3e495872536 round 47

## 1. Context / problem

The fixed upstream MTProto schema defines `Set_client_DH_params_answer` as an exact three-variant union:
`dh_gen_ok#3bcbf734 nonce:int128 server_nonce:int128 new_nonce_hash1:int128`,
`dh_gen_retry#46dc1fb9 nonce:int128 server_nonce:int128 new_nonce_hash2:int128`, and
`dh_gen_fail#a69dae02 nonce:int128 server_nonce:int128 new_nonce_hash3:int128`.

The existing canonical `telegram-protocol::AuthKeyHandshake` already consumes this response through
`parse_dh_gen_result` and `AuthKeyHandshake::receive_dh_gen`. It validates the active nonce pair,
checks the tag-specific new-nonce hash, rejects trailing bytes, establishes on OK, retries locally on RETRY,
and rejects on FAIL. The parser currently reads nonce/server_nonce/hash before constructor discrimination,
so a complete four-byte unknown constructor is misclassified as TL truncation rather than an unknown
`Set_client_DH_params_answer` variant.

## 2. Goal

Close the exact fixed-U dh_gen result/hash wire responsibility in the existing canonical owner by making the
constructor authoritative before field consumption, preserving exact nonce/hash validation and strict EOF,
and retaining the existing handshake-local retry transition without expanding into auth-key binding,
persistence, transport scheduling, or multi-request history.

## 3. Non-goals / out of scope

- Do not add a second handshake, protocol runtime, transport, account, or auth-key owner.
- Do not add or change temporary/permanent key binding, expiry enforcement, renewal, persistence, or teardown.
- Do not change connection probing/racing, plaintext send scheduling, message-id generation, timeout, or request history.
- Do not add gzip handling or complete updates difference/differenceSlice decoding/application.
- Do not claim broader DcKeyCreator temporary/persistent orchestration as closed by this slice.
- Do not run build, typecheck, lint, schema validation, generators, tests, packaging, deployment, installation,
  Actions, checks, or runtime acceptance in Stage A.

## 4. Requirements

- R1: Preserve fixed-U constructor constants `DH_GEN_OK_CONSTRUCTOR = 0x3bcbf734`,
  `DH_GEN_RETRY_CONSTRUCTOR = 0x46dc1fb9`, and `DH_GEN_FAIL_CONSTRUCTOR = 0xa69dae02`.
- R2: `parse_dh_gen_result` SHALL read the constructor first and reject any unknown constructor immediately
  as `HandshakeError::UnexpectedDhGenConstructor`, without attempting to consume nonce/hash fields.
- R3: For a recognized constructor, the parser SHALL read exactly nonce, server_nonce and one int128 hash,
  and validate nonce/server_nonce against the active handshake.
- R4: The expected hash SHALL be `SHA1(new_nonce || tag || auth_key_aux_hash_le)[4..20]`, using tag 1 for OK,
  tag 2 for RETRY and tag 3 for FAIL; comparison SHALL remain constant-time.
- R5: All three variants SHALL reject truncation and any trailing bytes after the exact fixed-U body.
- R6: `AuthKeyHandshake::receive_dh_gen` SHALL remain the only live handshake consumer and SHALL require
  `AuthKeyHandshakeState::AwaitingDhGen`.
- R7: OK SHALL consume the prepared key and transition to `Complete` using the existing server salt/time.
  RETRY SHALL remain handshake-local, prepare a new client-DH request with retry_id equal to the previous
  auth-key auxiliary hash, keep the state at `AwaitingDhGen`, and not create persistence/binding state.
  FAIL SHALL return `HandshakeError::DhGenRejected` without establishing a key.
- R8: The parser/transition SHALL remain synchronous and shall not own network scheduling, persistence,
  key binding/destruction, multi-request history, gzip, or updates-difference state.
- R9: Exact positive/negative contracts SHALL be authored in the existing protocol contract harness and
  mirrored into the existing desktop tracked provider contract surface without adding a desktop copy of
  `handshake.rs`. They SHALL NOT be executed during Stage A.

## 5. Current state

At platform-core checkpoint `8654e737a04900b6cac8c92660fa8a8b84b449bc`,
`native/telegram-protocol/src/handshake.rs@ed9eda7732f4753b442de98ebf510b8a7558b823`
contains the existing typed parser and `AuthKeyHandshake::receive_dh_gen`.
The parser already authenticates nonce/server_nonce, the tag-specific hash and EOF, while the existing
retry branch remains inside the same `AwaitingDhGen` owner. Only constructor ordering and complete contract
coverage remain open for this responsibility.

## 6. Target state

A four-byte unknown constructor deterministically fails with `UnexpectedDhGenConstructor`.
Each recognized variant validates the active nonce pair and only its corresponding hash tag, rejects all
truncation/trailing bytes, and returns an exact typed `DhGenResult`. The same `AuthKeyHandshake` performs
the existing OK/RETRY/FAIL transition, with RETRY remaining local and non-persistent.

## 7. Architecture and ownership boundaries

- `native/telegram-protocol/src/handshake.rs`: canonical fixed-U parser and AuthKeyHandshake state owner.
- `native/telegram-protocol/src/lib.rs`: existing public facade; no new duplicate wire owner.
- `native/telegram-protocol/tests/protocol_contract.rs`: authored-only Stage-A contracts.
- Desktop tracked telegram-protocol facade/contracts: synchronized consumer surface only; no copied handshake core.
- telegram-network/runtime/storage: unchanged by this slice.

## 8. Interfaces / contracts / schemas / data flow

`authenticated plaintext handshake response bytes -> parse_dh_gen_result(active nonces/new_nonce/aux hash) -> DhGenResult::{Ok,Retry,Fail} -> AuthKeyHandshake::receive_dh_gen`

OK returns the existing prepared auth key, salt and server time.
RETRY reuses the existing server-DH inputs and previous auxiliary hash as retry_id to prepare the next
`set_client_DH_params` request while retaining `AwaitingDhGen`.
FAIL terminates the current handshake attempt with `DhGenRejected`.

## 9. Constraints and non-functional requirements

- Fixed upstream U remains `telegramdesktop/tdesktop@f985723b552b30fe1c17d377d57f44e61a15a8c4`.
- Source schema blob remains `Telegram/SourceFiles/mtproto/scheme/mtproto.tl@76225965bfc794388db241f3f760db8ab79e84f8`.
- Shipping source anchor remains `Telegram/SourceFiles/mtproto/details/mtproto_dc_key_creator.cpp@13c57454f14b814b5923ea253048fb881ad8cf03`.
- Stage A implementation status may advance only to `implemented`, `validation_status=not-run`, `accepted=false`.

## 10. Failure modes and edge cases

- Unknown four-byte constructor is misclassified as field truncation.
- Known constructor accepts truncated nonce/server_nonce/hash.
- OK accepts hash2/hash3 material, RETRY accepts hash1/hash3 material, or FAIL accepts hash1/hash2 material.
- Nonce/server_nonce mismatch is accepted.
- Any variant accepts trailing bytes.
- RETRY escapes the existing handshake owner into persistence/binding/scheduler state.
- Desktop gains a second copied handshake implementation.

## 11. Implementation strategy

1. Commit this durable spec before behavior changes.
2. Reorder only constructor discrimination in the existing `parse_dh_gen_result`.
3. Author exact contracts for all three valid variants, four-byte unknown constructor, truncation cuts,
   nonce mismatch, server-nonce mismatch, variant/tag hash mismatch and trailing bytes.
4. Preserve `AuthKeyHandshake::receive_dh_gen` ownership and its existing handshake-local retry path.
5. Mirror the contract surface into the desktop Stage-A provider overlay; do not create desktop `handshake.rs`.
6. Update the existing parity ledger/P0/source-closure/evidence with source-level evidence only.

## 12. Verification / test strategy

Stage A: author and manually inspect contracts only. Do not execute them.
Stage B: after full migration closure, run the repository's real build/unit/contract/integration/package verification
against the frozen candidate and record artifact-bound evidence.

## 13. Acceptance criteria / Definition of Done

- AC-1: Unknown constructor discrimination happens before any field read.
- AC-2: All three fixed-U variants preserve exact nonce, tag-specific hash and no-trailing behavior.
- AC-3: Existing AuthKeyHandshake remains the only state owner and consumer.
- AC-4: RETRY remains local to `AwaitingDhGen` and does not add broader key lifetime/scheduler state.
- AC-5: Positive/negative contracts exist in canonical and desktop tracked contract surfaces and remain unexecuted.
- AC-6: Migration bookkeeping records exact commits/blobs with `validation_status=not-run` and `accepted=false`.

## 14. Release / migration / rollback

Stage A checkpoint branches only. No release or main integration in this slice. Rollback is the previous checkpoint ref.

## 15. Observability / evidence

Git commit identities, exact blob SHAs, source readback, and migration evidence files are the only Stage-A evidence.
No executable evidence is produced.

## 16. References / provenance

- `telegramdesktop/tdesktop@f985723b552b30fe1c17d377d57f44e61a15a8c4:Telegram/SourceFiles/mtproto/scheme/mtproto.tl@76225965bfc794388db241f3f760db8ab79e84f8`
- `telegramdesktop/tdesktop@f985723b552b30fe1c17d377d57f44e61a15a8c4:Telegram/SourceFiles/mtproto/details/mtproto_dc_key_creator.cpp@13c57454f14b814b5923ea253048fb881ad8cf03`
- Existing canonical `AuthKeyHandshake::receive_dh_gen` and `parse_dh_gen_result`.

## 17. Spec compliance record

| Requirement / AC | Status | Evidence / reason |
| --- | --- | --- |
| R1-R9 | pending | Spec committed before implementation. |
| AC-1-AC-6 | pending | Source-level compliance review follows implementation; executable validation remains deferred to Stage B. |
