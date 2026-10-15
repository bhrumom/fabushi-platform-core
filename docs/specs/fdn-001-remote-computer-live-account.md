# FDN Remote Computer live account authorization

Status: active; owner: Platform Core Remote Computer; 2026-10-15.
Parent: FDN-001 R07/R10/G08; session reconciliation and enrollment specs.

## Context, goal and current state
At Core f781e6c85375ba0945aa4fd4c19ead89e396dfdb, Remote Computer ordinary
account entrypoints use synchronous authenticated_account, which verifies RS256
claims but does not read session revocation. Logout/fixture cleanup can therefore
leave a still-signed token capable of re-registering devices until JWT expiry.
Use the existing authenticated_live_account owner before account-scoped reads or
mutations. Do not broaden factory cleanup while stale credentials can write back.

## Requirements
- LA01: Every ordinary-account Remote Computer entry and controller helper must
  await authenticated_live_account before reading/mutating the platform database.
  Reject absent/revoked/expired sessions and privileged legacy test identities.
- LA02: Preserve official MCP service fallback only where already supported;
  service delegation/session/scope and target-only deviceSecret fences remain.
  A failed ordinary token must not acquire service authority through fallback.
- LA03: Reuse canonical session SQL/RS256/time checks; no new issuer, tokens,
  tables, permissions or alternate auth logic. Internal network authority routes
  remain independent and must not accept ordinary or service credentials.
- LA04: All verification in Actions. Exact-head source guards, existing schema
  tests and wasm compilation must pass. Real isolated cloud consumer must prove
  old token cannot list/register/heartbeat after logout and cleanup; contracts
  alone are not runtime or production acceptance.

## Architecture and implementation
Remote Computer remains the platform resource owner; account.rs owns ordinary
live-session auth. Change only ordinary calls in remote_computer.rs to the async
helper. Update existing source assertions to require the stronger check, including
internal-route forbidden calls, and add a global guard against signature-only
ordinary auth. Preserve device/client/session/operation/reconcile code and schemas.

## Failure modes, verification and acceptance
Revoked/deleted/expired/missing sessions fail401 before resource work. Missing D1
fails closed. Wrong issuer/audience remains denied. Service credentials continue
through their own verified delegation path. No compatibility acceptance for the
legacy privileged test shortcut on these security-critical resources.
AC1: LA01–03 source/wasm/schema gates pass exact PR and merged main. AC2: LA04
real ordinary-session revocation proof using isolated factory and canonical wasm.
AC3: production canonical deployment and runtime proof are separate; do not mark
G08/G16 passed from contract CI. Missing environment remains blocked.

## Delivery, rollback, observability and provenance
Protected Core PR then postmerge Actions; Backend canonical delivery must pin
verified new Core main. No schema migration. Retain auth failure status/body
redaction and avoid token/error-body logging. Rollback must not knowingly restore
signature-only authorization; stop affected acceptance/cutover on regression.
User asks finishing real production/browser acceptance2026-10-15. Discovery:
remote_computer.rs ordinary auth and account.rs authenticated_live_account at
f781e6c85375ba0945aa4fd4c19ead89e396dfdb; factory AF05 ownership cleanup.
