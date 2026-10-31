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
| R1-R9 | implemented / not-run | Protocol commit 314e8804ddfc997acdde2c19bff831b5e26b1706, encrypted storage commit bcb8298a4622616f2f8a226899c0e3dc8aa3ba8f, and runtime commit ec854e93fb1523d95225203554600858798e4981 implement the specified owners. Executable validation was intentionally not run in Stage A. |
| R10 | implemented | Contract tests are authored in protocol/storage/runtime test owners and remain unexecuted. |
| AC-1-AC-3 | implemented / not-run | Runtime/protocol/storage source is committed on the isolated checkpoint; no executable verification is claimed. |
| AC-4 | implemented / not-run | Desktop checkpoint 169d6cc7798507f0302d6c93fef7deef1a302d28 wires all seven commands through the existing TelegramClientLease and feature.auth chain. |
| AC-5-AC-6 | implemented / not-run | Test execution evidence remains empty; migration ledger/evidence update is performed in the desktop checkpoint and acceptance remains false. |

## 18. Stage-A implementation evidence

Canonical platform-core checkpoint: `checkpoint/tdrp-telegram-auth-lifecycle-7457eb1c-r16`.

- protocol source: `native/telegram-protocol/src/api.rs@f680705bd87a26e39e9fb0d9bd83337e338512f3`
- protocol exports: `native/telegram-protocol/src/lib.rs@fd74519aefe597ff127fa9a2b8067eaa4d8cdb01`
- protocol authored tests: `native/telegram-protocol/tests/protocol_contract.rs@4d6bec0d84016fc7651c69985ad19f005f22eaa1`
- encrypted runtime-state storage: `native/telegram-storage/src/sqlite.rs@b07f8e95efba8a2f910f4d85345deb2619c23a84`
- storage authored tests: `native/telegram-storage/tests/encrypted_store.rs@9103d6d7118f82c7252d27e771a9c352c6b4f548`
- runtime commands/persistence: `native/telegram-runtime/src/runtime.rs@adb9d29f352cc8e8a62ecdb90e77d546d78b05b3`
- runtime authored tests: `native/telegram-runtime/tests/runtime_ffi.rs@fcb63dc714a05337732e4318aaaa24f8f582ef3a`

No build, typecheck, lint, schema validator, generator, unit/contract/integration/E2E test, packaging, deployment, installation, workflow/status query, or runtime acceptance was executed in Stage A.
