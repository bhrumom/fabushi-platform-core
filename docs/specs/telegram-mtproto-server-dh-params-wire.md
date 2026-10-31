# Telegram MTProto server_DH_params wire contract

Status: active  
Owner: shared telegram-protocol / AuthKeyHandshake  
Last updated: 2026-11-01  
Related issue/task/PR: Fabushi migration task 7457eb1c-11a3-4626-8563-a3e495872536 round 46

## 1. Context / problem

The fixed upstream MTProto schema defines `Server_DH_Params` as an exact two-variant union:
`server_DH_params_ok#d0e8075c nonce:int128 server_nonce:int128 encrypted_answer:string` and
`server_DH_params_fail#79cb045d nonce:int128 server_nonce:int128 new_nonce_hash:int128`.

The existing canonical `telegram-protocol::AuthKeyHandshake` already consumes this response through
`parse_server_dh_params` and owns the surrounding auth-key handshake state. The parser currently reads
the shared nonce fields before it discriminates the constructor. As a result, a complete four-byte unknown
constructor is reported as a truncated TL body rather than as an unknown `Server_DH_Params` variant.
The union discriminator should be authoritative before variant fields are read.

## 2. Goal

Harden the existing canonical `Server_DH_Params` parser so fixed-U constructor discrimination is exact and
fail-closed before variant fields are consumed, while preserving the existing nonce validation, payload framing,
no-trailing rule, and `AuthKeyHandshake::receive_server_dh` ownership.

## 3. Non-goals / out of scope

- Do not add a second handshake, protocol runtime, transport, account, or auth-key owner.
- Do not expand auth-key destruction, temporary/permanent key binding, expiry enforcement, renewal, or persistence.
- Do not change connection probing/racing, plaintext send scheduling, message-id generation, retry/timeout, or request history.
- Do not claim `server_DH_inner_data` decryption/hash/DH-prime/g_a validation as newly implemented by this slice.
- Do not add gzip handling or complete updates difference/differenceSlice decoding/application.
- Do not run build, typecheck, lint, schema validation, generators, tests, packaging, deployment, installation, Actions, checks, or runtime acceptance in Stage A.

## 4. Requirements

- R1: Preserve fixed-U constructor constants `SERVER_DH_PARAMS_OK_CONSTRUCTOR = 0xd0e8075c` and `SERVER_DH_PARAMS_FAIL_CONSTRUCTOR = 0x79cb045d`.
- R2: `parse_server_dh_params` SHALL read the constructor first and reject any unknown constructor immediately as `HandshakeError::UnexpectedServerDhConstructor`, without attempting to consume nonce or payload fields.
- R3: For a recognized constructor, the parser SHALL read exactly nonce and server_nonce, validate them against the active handshake, and only then read that variant's payload.
- R4: The OK variant SHALL decode exactly one canonical TL bytes/string `encrypted_answer`; malformed/truncated bytes framing SHALL fail closed.
- R5: The FAIL variant SHALL decode exactly one raw int128 `new_nonce_hash`; truncation SHALL fail closed.
- R6: Both variants SHALL reject trailing bytes after the exact fixed-U body.
- R7: `AuthKeyHandshake::receive_server_dh` SHALL continue to consume this same parser. No second state owner or disconnected parser path may be introduced.
- R8: The parser SHALL remain synchronous and side-effect free with respect to network send, auth-key persistence/deletion/binding, session teardown/reconnect, multi-request history, gzip, and updates difference handling.
- R9: Exact positive/negative contracts SHALL be authored in the existing protocol contract harness and mirrored into the existing desktop tracked provider contract surface without adding a desktop copy of `handshake.rs`. They SHALL NOT be executed during Stage A.

## 5. Current state

At platform-core checkpoint `42c7ac4d193f7f1872a9bfd87ce70b2af973c2df`,
`native/telegram-protocol/src/handshake.rs@3e740e0714dfca71bb908ec46f182e98605582e5`
contains the existing typed parser and `AuthKeyHandshake::receive_server_dh`. The parser validates recognized
variants and trailing bytes, but constructor discrimination occurs after common nonce fields are read.

The desktop Stage-A checkpoint tracks the canonical protocol public facade and contract harness, but intentionally
does not carry a second `handshake.rs` implementation. That existing-owner boundary SHALL remain unchanged.

## 6. Target state

A four-byte unknown constructor deterministically fails with `UnexpectedServerDhConstructor`. Recognized OK and
FAIL bodies retain exact nonce binding, exact payload decoding, and no-trailing validation. The same
`AuthKeyHandshake::receive_server_dh` remains the only live handshake consumer.

## 7. Architecture and ownership boundaries

- `native/telegram-protocol/src/handshake.rs`: canonical fixed-U parser and AuthKeyHandshake state owner.
- `native/telegram-protocol/src/lib.rs`: existing public facade; no new duplicate wire owner.
- `native/telegram-protocol/tests/protocol_contract.rs`: authored-only Stage-A contracts.
- Desktop tracked telegram-protocol facade/contracts: synchronized consumer surface only; no copied handshake core.
- telegram-network/runtime/storage: unchanged by this slice.

## 8. Interfaces / contracts / schemas / data flow

`authenticated plaintext handshake response bytes -> parse_server_dh_params(expected nonce pair) -> ServerDhParams::{Ok,Fail}`

For OK, `encrypted_answer` remains opaque to this outer parser and is handed to the existing
`decrypt_server_dh_inner_data` path by `AuthKeyHandshake::receive_server_dh`.
For FAIL, the existing caller authenticates `new_nonce_hash` against the current new_nonce.

## 9. Constraints and non-functional requirements

- Fixed upstream U remains `telegramdesktop/tdesktop@f985723b552b30fe1c17d377d57f44e61a15a8c4`.
- Source schema blob remains `Telegram/SourceFiles/mtproto/scheme/mtproto.tl@76225965bfc794388db241f3f760db8ab79e84f8`.
- Shipping source anchor remains `Telegram/SourceFiles/mtproto/details/mtproto_dc_key_creator.cpp@13c57454f14b814b5923ea253048fb881ad8cf03`.
- Stage A implementation status may advance only to `implemented`, `validation_status=not-run`, `accepted=false`.

## 10. Failure modes and edge cases

- Unknown four-byte constructor is misclassified as field truncation.
- Known constructor with truncated nonce/server_nonce/payload is accepted.
- OK payload accepts malformed TL bytes padding/length.
- FAIL payload accepts fewer than 16 hash bytes.
- Either variant accepts trailing bytes.
- Parser mutates handshake/network/persistence state directly.
- Desktop gains a second copied handshake implementation.

## 11. Implementation strategy

1. Commit this durable spec before behavior changes.
2. Reorder only constructor discrimination in the existing `parse_server_dh_params`.
3. Add exact authored contracts for OK, FAIL, unknown constructor, truncation, nonce mismatch, server-nonce mismatch, and trailing bytes.
4. Keep the existing public facade and AuthKeyHandshake consumer unchanged.
5. Mirror the contract surface into the desktop Stage-A provider overlay; do not create desktop `handshake.rs`.
6. Update the existing parity ledger/P0/source-closure/evidence with source-level evidence only.

## 12. Verification / test strategy

Stage A: author and manually inspect contracts only. Do not execute them.
Stage B: after full migration closure, run the repository's real build/unit/contract/integration/package verification
against the frozen candidate and record artifact-bound evidence.

## 13. Acceptance criteria / Definition of Done

- AC-1: Unknown constructor discrimination happens before any variant/common field read.
- AC-2: Both fixed-U variants preserve exact nonce/payload/no-trailing behavior.
- AC-3: Existing AuthKeyHandshake remains the only state owner and consumer.
- AC-4: No out-of-scope lifecycle or scheduler state is introduced.
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
- Existing canonical `AuthKeyHandshake::receive_server_dh`.

## 17. Spec compliance record

| Requirement / AC | Status | Evidence / reason |
| --- | --- | --- |
| R1-R9 | pending | Spec committed before implementation. |
| AC-1-AC-6 | pending | Source-level compliance review follows implementation; executable validation remains deferred to Stage B. |
