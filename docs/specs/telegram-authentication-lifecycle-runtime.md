# Telegram authentication lifecycle runtime primitives — Specification

Status: active  
Owner: shared Telegram protocol/runtime  
Last updated: 2026-11-01  
Related issue/task/PR: Fabushi migration task 7457eb1c-11a3-4626-8563-a3e495872536 round 16

## 1. Context / problem

The canonical shared Telegram protocol/runtime currently supports send-code, sign-in, password submission, registration, and account.getPassword during SESSION_PASSWORD_NEEDED. The fixed Telegram Desktop upstream also owns resend/cancel, password-recovery validation, login-email verification, and bounded SRP challenge refresh. Desktop already exposes one process-lifetime Telegram client lease and must not create a second account/runtime.

## 2. Goal

Add source-faithful protocol primitives and runtime commands for the remaining login lifecycle while preserving one client/session owner and encrypted persistence for in-progress authorization context.

## 3. Non-goals / out of scope

- No second Telegram runtime, account, provider, or network session.
- No TDLib, Telegram subprocess, or original C++ business owner.
- No automatic password retry after SRP_ID_INVALID; only a safe challenge-refresh primitive.
- No password reset/new-password settings flow after a recovery code is checked.
- No CI, build, lint, generator, test execution, packaging, deployment, or runtime acceptance in Stage A.

## 4. Requirements

- R1: Protocol builders SHALL encode fixed-U constructors for auth.resendCode, auth.cancelCode, auth.requestPasswordRecovery, auth.checkRecoveryPassword, account.sendVerifyEmailCode, account.verifyEmail, and account.getPassword.
- R2: Protocol parsers SHALL decode auth.PasswordRecovery, account.SentEmailCode, Bool, and the login-relevant account.EmailVerified variants including nested auth.SentCode.
- R3: Runtime SHALL expose commands telegram.resendAuthenticationCode, telegram.cancelAuthenticationCode, telegram.requestPasswordRecovery, telegram.checkRecoveryPassword, telegram.sendVerifyEmailCode, telegram.verifyEmail, and telegram.refreshPasswordChallenge on the existing client id and transport.
- R4: resend SHALL replace the current phone_code_hash with the returned hash; cancel SHALL clear the in-progress auth context and project WaitPhoneNumber.
- R5: requestPasswordRecovery and checkRecoveryPassword SHALL keep authorization in WaitPassword; checkRecoveryPassword validates the code only and SHALL NOT mark the account Ready.
- R6: sendVerifyEmailCode SHALL use emailVerifyPurposeLoginSetup(phone_number, phone_code_hash). verifyEmail SHALL consume account.emailVerifiedLogin; nested auth.sentCode SHALL update the current phone_code_hash and nested auth.sentCodeSuccess SHALL project Ready. Plain account.emailVerified is invalid for this login-setup flow.
- R7: telegram.refreshPasswordChallenge SHALL invoke account.getPassword, replace cached SRP parameters, and return a refreshed WaitPassword projection without retrying the submitted password.
- R8: Persistent runtimes SHALL store the authorization state and AuthContext in the existing encrypted Telegram store, restore them on reopen, and clear the in-progress context after Ready or cancel. Sensitive password plaintext SHALL never be persisted.
- R9: Existing protocol/core/message persistence owners remain unchanged except for a minimal encrypted runtime-state slot in telegram-storage.
- R10: Stage A verification artifacts SHALL be authored but not executed.

## 5. Current state

At canonical platform-core main 26c3cc8c69ab89e8fb38952adb8c438ec0b68e05, native/telegram-protocol implements sendCode/signIn/signUp/checkPassword/account.getPassword; native/telegram-runtime dispatches only sendAuthenticationCode/submitAuthenticationCode/submitAuthenticationPassword/submitRegistration for login lifecycle. AuthContext is process-memory only.

## 6. Target state

The same runtime client and established transport implement the seven additional lifecycle commands. The encrypted store can persist a bounded authorization runtime record independently of Telegram message/domain state.

## 7. Architecture and ownership boundaries

- native/telegram-protocol: TL constructors, builders, parsers only.
- native/telegram-runtime: command dispatch, authorization state transitions, one transport/session, AuthContext lifecycle.
- native/telegram-storage: generic encrypted runtime-state record; no Telegram auth policy.
- Desktop/application layer: calls these commands through its existing TelegramClientLease and feature.auth contract.

## 8. Interfaces / contracts / schemas / data flow

Runtime request @type values are listed in R3. Email verification input fields are email/code. Recovery check input is code. Resend/cancel use the current AuthContext phone number/hash and do not accept a caller-supplied hash. Challenge refresh has no password input.

Encrypted runtime state contains AuthorizationState plus phone_number, phone_code_hash, and optional PasswordSrpParameters. It is encrypted with the same StorageCipher using domain-separated associated data.

## 9. Constraints and non-functional requirements

- No api_hash is returned from the runtime.
- Zeroizing remains used for in-memory phone/hash copies.
- Remote RPC errors remain structured and are not converted to success.
- All commands restore the same EstablishedSession to the same RuntimeClient after an invocation.
- Fixed upstream U is f985723b552b30fe1c17d377d57f44e61a15a8c4.

## 10. Failure modes and edge cases

- Missing auth context: auth_context_missing.
- Missing transport: transport_not_connected.
- Invalid email/recovery code inputs: invalid_parameter or protocol validation failure.
- RPC errors including SRP_ID_INVALID remain telegram_rpc_error; refresh is a separate explicit primitive.
- account.EmailVerified without the login variant is a protocol error in this flow.
- Storage corruption/decryption failures fail closed.

## 11. Implementation strategy

1. Add fixed-U TL constants/builders/parsers and authored contract tests.
2. Add encrypted secure runtime-state storage and authored migration/storage tests.
3. Add runtime commands, helpers, persistence/restore, and authored runtime tests.
4. Wire desktop TelegramClientLease and feature.auth to the new runtime commands in the desktop repository.
5. Update migration ledger/P0/source-closure/evidence with validation_status=not-run and accepted=false.

## 12. Verification / test strategy

Stage A only authors protocol/storage/runtime contract tests and desktop contract tests. No test or validator is executed. Stage B will run exact-head verification after full migration closure.

## 13. Acceptance criteria / Definition of Done

- AC-1: All seven runtime lifecycle commands are implemented through the existing client/session owner.
- AC-2: Builders/parsers match fixed-U api.tl constructors and login semantics.
- AC-3: In-progress authorization state/context survives persistent-client reopen through encrypted storage.
- AC-4: Desktop canonical feature.auth calls the new commands through the same TelegramClientLease.
- AC-5: Auth lifecycle tests exist but Stage A execution evidence remains empty/not-run.
- AC-6: Migration ledgers/evidence identify exact commits/blobs and do not claim verified/accepted.

## 14. Release / migration / rollback

Stage A checkpoint branches only. No release or main integration in this round. Rollback is the prior checkpoint/ref because storage schema changes are additive.

## 15. Observability / evidence

Repository commits and exact blob SHAs are the Stage A evidence. Runtime RPC errors retain Telegram code/message for desktop classification. No executable evidence is produced in Stage A.

## 16. References / provenance

- telegramdesktop/tdesktop@f985723b552b30fe1c17d377d57f44e61a15a8c4 Telegram/SourceFiles/mtproto/scheme/api.tl
- intro/intro_code.cpp
- intro/intro_password_check.cpp
- intro/intro_email.cpp

## 17. Spec compliance record

| Requirement / AC | Status | Evidence / reason |
| --- | --- | --- |
| R1-R9 | implemented / not-run | Original lifecycle primitives remain present and are fused into the canonical identity/transport runtime through commits 6b1a75a74f5ee2b100f70d905051bf961a040585, b2a45edb2e4e3e62a1543410e9ad87f4c1a87e2a and 4fbff4f54caed03b7b1ba68efe2f008fa4447622. Executable validation was intentionally not run in Stage A. |
| R10 | implemented | Contract tests are authored in protocol/storage/runtime test owners and remain unexecuted; historical-v3 migration shapes are corrected in 13f5458aadf19a87c52bfa904f9942d81c88979a. |
| R11-R13 | implemented / not-run | AuthorizationSnapshot/self_user_id, auth.Authorization identity parsing, resumable transport, serializable SRP challenge state and unified storage schema v4 are committed on this checkpoint. |
| R14 | implemented / not-run | Desktop checkpoint 43d9d398801f2885da982d106c17a74efb1f9472 carries the exact Mahayana workspace/path manifests and synchronized providers/telegram-* production blobs; authored integration contracts continue through 221a661e17f2ad417e349df6604815d3fbfa946f. |
| R15 | implemented | Composition, runtime, protocol and both historical v3 storage migration contracts are authored only; no executable evidence exists yet. |
| AC-1-AC-6 | implemented / not-run | Production and ledger/source-closure evidence is committed; acceptance remains false until Stage B executable verification and independent review. |

## 17A. Composition authority reconciliation (round 17)

Current repository evidence establishes that the shipping desktop Cargo authority is the `source/mahayana/mahayana-rs` workspace in `bhrumom/fabushi-desktop`: the workspace lists `mahayana-telegram` and `providers/telegram-runtime`; `mahayana-telegram/Cargo.toml` depends on `fabushi-telegram-runtime = { path = "../providers/telegram-runtime" }`; and the runtime path-depends on sibling protocol, network, storage, and core crates. The isolated Stage-A desktop checkpoint is a sparse overlay and omits these Cargo/provider files, so that sparse tree alone is not composition authority.

The current desktop migration lineage also contains the later authorization/identity baseline at `bhrumom/fabushi-desktop@d19fc3742b9a2d5076513ec444383d7683a35ea2`: runtime blob `95d27ee01f5cf1da15f595ab54e3660b24321128`, protocol API blob `c383b3ebb1eade1dd1b4eb1686cec04d6856867e`, network blob `520bf4e0e2094e74c878ad1f32f3eb32b6edae32`, and storage blob `84387005dc1d51962980a4cb4c6ce69e0b308695`. That baseline introduced `AuthorizationSnapshot { state, self_user_id }`, persisted canonical self-user identity, resumable MTProto auth-key transport, and live `authorization_snapshot()` handoff. Those responsibilities must be preserved while adding this specification's lifecycle primitives.

Storage has a historical schema-number collision: the identity/transport baseline used schema v3 for encrypted `private_metadata`, while this lifecycle checkpoint independently used schema v3 for encrypted `secure_value`. The fused authority SHALL move to schema v4, contain both encrypted tables, and migrate either recognized v3 shape forward without discarding data or treating one historical shape as proof that the other table exists.

Additional requirements:

- R11: the fused runtime SHALL preserve `authorization_snapshot()`, canonical `self_user_id`, persisted resumable transport, and the single-client lease semantics from the existing desktop baseline.
- R12: `auth.sentCodeSuccess`, successful password authentication, registration, and login-email success SHALL parse the returned `auth.Authorization` user and settle Ready with its canonical self user id; Ready without canonical identity or resumable transport remains fail-closed.
- R13: the storage schema SHALL be v4 and support migration from both recognized v3 layouts (`private_metadata`-only and `secure_value`-only), preserving encrypted contents while adding the missing table.
- R14: the actual desktop Cargo provider path SHALL receive the same fused protocol/network/storage/runtime implementation so the shipping workspace consumes the unique runtime behavior rather than a disconnected parallel copy.
- R15: integration/migration contracts for composition, identity preservation, lifecycle persistence, and v3-to-v4 compatibility SHALL be authored only; executable evidence remains not-run in Stage A.

## 18. Stage-A implementation evidence

Canonical platform-core checkpoint: `checkpoint/tdrp-telegram-auth-lifecycle-7457eb1c-r16`.

- composition/spec reconciliation commit: `399f0fdf4d56e8c129118825a34f7d1aa13b6dc1`
- protocol identity/lifecycle source: `native/telegram-protocol/src/api.rs@751219099926a809d455d31b005c48a841d879f2`
- protocol exports: `native/telegram-protocol/src/lib.rs@34bcd749f758cfd3f562b6e701ee10eb51d5d0c2`
- serializable SRP challenge state: `native/telegram-protocol/src/srp.rs@06ff7c4ed54f445c9df8bf96deb1deb7defe4939`
- transport resume primitive: `native/telegram-network/src/lib.rs@24e236df503e273490e44645251d757e30712ea3`
- unified encrypted schema-v4 storage: `native/telegram-storage/src/sqlite.rs@e6cc314ec749f8a258bfe0125a25beb3613eb2ca`
- both-v3-shape authored storage contracts: `native/telegram-storage/tests/encrypted_store.rs@ecde6882fc7265eee0f1c2714a8eb4fa1543d63f`
- fused runtime identity/transport/lifecycle owner: `native/telegram-runtime/src/runtime.rs@10031c5261bb0810ad2df2fca3de2e2fe4f65301`
- runtime public projection: `native/telegram-runtime/src/lib.rs@0c79707ba1e0932de15fad3c57ad00fcb9682aff`
- runtime manifest with persisted-state serde support: `native/telegram-runtime/Cargo.toml@a0d5aee63c886b11c3ad8fbf37885e0a3c5b6053`
- authored runtime contracts: `native/telegram-runtime/tests/runtime_ffi.rs@97eb341731e6122f615aeac2aba5eaa12d3a9fbd`
- current implementation/test checkpoint before this evidence-only spec update: `13f5458aadf19a87c52bfa904f9942d81c88979a`
- desktop static Cargo/provider composition: `bhrumom/fabushi-desktop@43d9d398801f2885da982d106c17a74efb1f9472`
- desktop authored composition/storage/runtime contracts through: `bhrumom/fabushi-desktop@221a661e17f2ad417e349df6604815d3fbfa946f`
- desktop ledger/P0/source-closure evidence commit: `bhrumom/fabushi-desktop@3f78d4d44d7d5cc9c1892b2ec2cdd945e2367a66`

No build, typecheck, lint, schema validator, generator, unit/contract/integration/E2E test, packaging, deployment, installation, workflow/status query, or runtime acceptance was executed in Stage A.
