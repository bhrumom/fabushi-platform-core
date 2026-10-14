# Account information session revocation — Specification
Status: active; owner canonical Core account API; updated 2026-10-14.

## 1. Context / 2. Goal
FCM-027 explicitly requires real account refresh/logout/revocation tests. At
main 0ba62a11bd20c36785fb8bb1c982ae6fcdba2d16 account_user_info verifies only
RS256 claims via synchronous authenticated_account. Logout writes revoked_at,
but account information does not query that session. Goal: ordinary account
information and logout require a current server-owned account session.

## 3. Non-goals
No new account issuer/table/key/password algorithm. Do not rewrite all unrelated
marketplace/listener/commerce/Remote Computer endpoints in this narrow fix; those
remain separate audit/acceptance obligations. Do not claim global revoke safety
from this route regression. Legacy privileged TestAccount cannot be used here.

## 4. Requirements
ASR-1: /api/auth/user-info requires verified ordinary RS256 account identity AND
an existing account_sessions row bound to the token's sid/user, unrevoked and
unexpired according to server time. Unknown/revoked/expired/mismatched session or
special TestAccount is rejected before returning account data.
ASR-2: /api/auth/logout uses the same live-session gate before revoking the exact
session. DB/configuration errors fail closed; no fallback to signature-only auth.
ASR-3: Normal registration/login/refresh keep their canonical algorithms/contracts.
No old credentials/session state is rewritten. Other user sessions stay valid.
ASR-4: Actual wasm Actions integration proves fresh user-info, logout then old
access/refresh rejection, missing session rejection and same-account second
session remains valid. Exact-source compile/regression evidence is required.

## 5. Current / 6. Target
Current signature-only route; target signature + authoritative session lookup.
Existing session_id, user_id, expires_at, revoked_at fields suffice.

## 7. Architecture / 8. Contract
Extend existing worker_api/account.rs with an async live account helper. Sync
signature helper remains for existing owners; this scope does not declare them
revocation safe. Auth failure status 401; missing configuration/storage remains
closed 401 at these routes. No client-supplied session selector is trusted.

## 9. Constraints / 10. Failures
Actions-only builds/tests. No production account writes or deployment. Expiry at
now is rejected. Deleting a session rejects its old signed token. Query binds
both session and user. A transient storage failure must never return private data.

## 11. Strategy / 12. Verification
Spec before code; add helper to the existing account owner; wire user-info/logout;
run existing native policy and actual wasm checks, and Backend's pinned real
canonical Worker integration on the exact Core SHA. Never weaken its revoke
assertion to accommodate current behavior.

## 13. Acceptance
AC-1: ASR-1..3 exact-source Core Actions compile and existing regressions pass.
AC-2: ASR-4 real Worker runtime assertions pass, including fresh/other session.
AC-3: PR integration and canonical main readback recorded; production deployment
is a separate Backend delivery gate, not implied by a Core merge.

## 14. Migration / rollback / 15. Evidence
No migration. Verified Core source may later deploy through canonical Backend.
Rollback reopens the defect and therefore blocks affected revocation acceptance.
Retain Core/Backend exact SHA, Actions run, assertion counts and wasm digest;
never credentials or raw login responses in artifacts.

## 16. Provenance
User next-generation testing proposal dated 2026-10-14; canonical Backend
isolated-ci-account-factory spec AF-08/AC-5; Core account.rs auth/logout/user-info
and account-migrations/0001_account_auth.sql; AGENTS.md spec-first policy.

## 17. Compliance
| Criterion | Status | Evidence / reason |
| --- | --- | --- |
| ASR-1..3 / AC-1 | blocked | Actions compilation pending |
| ASR-4 / AC-2 | blocked | Exact-Core runtime pending |
| AC-3 | blocked | PR integration pending; no production deployment claimed |
