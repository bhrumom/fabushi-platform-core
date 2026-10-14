# FDN-001 Remote Computer Operation Journal

Status: implementation
Owner: Fabushi Platform Core / Remote Computer control plane
Parent authority: bhrumom/fabushi-desktop/docs/specs/fabushi-same-account-device-network.md (FDN-001)

## Goal

Provide one durable, account-scoped operation identity and result-reconciliation owner for side-effecting remote device calls. Network timeout, DERP/reverse-WSS loss, or caller process death must never be interpreted as proof that an operation did not execute.

This extends the existing Remote Computer session/grant authority. It does not create another device registry, another authorization system, or a Backend-owned operation database.

## Non-goals

- This change does not transport tool arguments or sensitive-input plaintext through D1.
- This change does not authorize a caller merely because it shares a Fabushi account.
- This change does not make Tailscale/WireGuard state an application authorization signal.
- This change does not replace the existing Remote Computer session, grant, revocation, or target-device activation rules.
- This change does not claim FDN-A07/F16 end-to-end completion without real device/transport evidence.

## Requirements

### OPJ-01 Stable request identity

A controller creates an operation with a caller-supplied stable `requestId` (16..160 bounded opaque identifier) under exactly one authenticated account, device, active Remote Computer session, and paired client. The tuple `(user, device, session, requestId)` is unique.

### OPJ-02 Request digest fencing

The journal stores a SHA-256 request digest and bounded tool identifier, never arbitrary tool arguments. Reusing a `requestId` with a different tool or digest is a conflict. A duplicate with the same identity converges to the same operation record.

### OPJ-03 Existing session/grant authority

Creating/querying/cancelling an operation as controller requires the existing active session mobile actor proof (`clientId` + `mobileToken`) and a non-revoked session grant. Target result publication requires the existing target device secret and the same active session/grant. Account/device/session/client mismatches fail closed.

### OPJ-04 Durable states

States are `pending`, `running`, `completed`, `failed`, `outcome_unknown`, or `cancelled`.

Terminal states never transition back to executable states. A caller may mark a sent request `outcome_unknown` after losing its authenticated transport response. That state is queryable and must not be auto-retried as a new operation.

### OPJ-05 Result recovery

The target can publish a bounded result envelope after authenticating with the target device secret. Normal results may contain bounded JSON needed for reconciliation. Calls explicitly marked `sensitiveResult=true` never persist result plaintext; only status and SHA-256 digest are retained.

### OPJ-06 Cancellation

The controller can cancel only `pending` operations. `running` or `outcome_unknown` cannot be rewritten as cancelled because execution may already have happened. Target-side execution must still honor its local cancellation/authorization fencing.

### OPJ-07 Revocation and expiry

Closing/expiring the session or revoking its client/grant prevents new operation creation and prevents stale actors from updating/querying protected operation results. Existing audit/journal rows may remain for bounded reconciliation/audit retention, but do not restore authorization.

### OPJ-08 Bounds and privacy

Tool names and request IDs are bounded identifiers. Request body is represented only by its digest. Result JSON is capped at 256 KiB. No password, OTP, API key, device secret, mobile token, client token, Fabushi login token, network private key, or secure-input payload is persisted in the journal.

### OPJ-09 Audit

Creation and every state transition append an operation audit event with account/device/client/session/operation metadata, requestId, tool name, prior/new state and timestamp. Audit rows do not contain tool arguments or result plaintext.

### OPJ-10 Backward-compatible integration

Official MCP will consume these APIs from the canonical Backend service. Existing `fabushi_account`, `list_devices`, `describe_device_tool`, and `device_call` remain compatible; stable request identity/result query are additive extensions.

## API contract

Controller create:

`POST /v1/computers/:device_id/sessions/:session_id/operations`

Body:
- `clientId`
- `mobileToken`
- `requestId`
- `toolName`
- `requestSha256` (64 lowercase hex)
- `sensitiveResult` (default false)

Returns the converged operation record.

Controller query:

`POST /v1/computers/:device_id/sessions/:session_id/operations/:request_id/query`

Body: `clientId`, `mobileToken`.

Controller outcome-unknown:

`POST /v1/computers/:device_id/sessions/:session_id/operations/:request_id/outcome-unknown`

Body: `clientId`, `mobileToken`. Only pending/running may become outcome_unknown.

Controller cancel:

`POST /v1/computers/:device_id/sessions/:session_id/operations/:request_id/cancel`

Body: `clientId`, `mobileToken`. Only pending may become cancelled.

Target claim:

`POST /v1/computers/:device_id/sessions/:session_id/operations/:request_id/claim`

Body: `deviceSecret`. Only pending may become running.

Target result:

`POST /v1/computers/:device_id/sessions/:session_id/operations/:request_id/result`

Body:
- `deviceSecret`
- `status`: completed | failed
- `resultSha256`: 64 lowercase hex
- optional `result` JSON; rejected for sensitiveResult operations and capped at 256 KiB serialized.

## Migration

Add `remote_computer_operations` keyed to Remote Computer session identity and `remote_computer_operation_audit`. Use migration 0028. No reset/rename of existing Remote Computer tables.

## Verification

GitHub Actions only.

- Rust format/check/test through the canonical deploy/platform-worker manifest.
- Unit tests for requestId/hash/state transition validation.
- Schema/contract tests verify migration tables, unique identity, terminal-state monotonicity, sensitive-result plaintext exclusion, and routes.
- Negative cases: cross-account/session mismatch, revoked client/grant, wrong mobile token, wrong device secret, requestId digest conflict, illegal terminal transition, oversized result.

## Acceptance

- OPJ-A1: exact-head Actions prove migration + Rust contract compile/test.
- OPJ-A2: duplicate same request identity converges; changed digest conflicts.
- OPJ-A3: timeout/result-loss can be represented as outcome_unknown and queried without creating a second operation.
- OPJ-A4: controller/target authentication uses existing Remote Computer actors and grants.
- OPJ-A5: sensitiveResult journal rows contain no plaintext result.
- OPJ-A6: session/client/grant revocation fences further access/state changes.

Real official-MCP-to-device FDN-A07/A09/A16 evidence remains pending until Backend/Host/network integration and deployed device acceptance are complete.
