# FDN-001 Remote Computer session reconciliation — Specification

Status: active  
Owner: Platform Core Remote Computer worker  
Last updated: 2026-10-14  
Related: Fabushi Desktop FDN-001; Desktop PR #65 merged as 9fe57d7e23bc11d9a05e08519becc3761accf22e

## 1. Context / problem

FDN-001 requires stable request identity and reconciliation for Remote Computer session creation. The current production request accepts only `clientId` and `clientToken` with `deny_unknown_fields`, creates a fresh session and mobile token on every accepted request, and therefore cannot safely recover when the create response is lost.

## 2. Goal

Add a backwards-compatible, account-scoped session creation contract with optional stable `requestId` and explicit `reconcile` recovery. A retried request must never create a second session for the same stable request identity. Recovery must keep session credentials separate from the paired-client credential.

## 3. Non-goals / out of scope

This change does not implement the tsnet worker, Headscale, DERP, official MCP gateway, native computer control, or full FDN-001 operation journal. It does not weaken existing client pairing, device-secret, account, revocation, or session-state checks.

## 4. Requirements

- FDN-CORE-R1: Accept optional `requestId` and optional `reconcile` while preserving legacy clients that omit both.
- FDN-CORE-R2: When present, `requestId` is validated as a bounded relay identifier and is unique per authenticated Fabushi account.
- FDN-CORE-R3: The same account/requestId cannot be reused for another device or client.
- FDN-CORE-R4: A duplicate stable request does not insert a second session.
- FDN-CORE-R5: Explicit `reconcile=true` on a live matching session returns the same `sessionId` and rotates/reissues a new mobile session credential; it does not derive that credential from `clientToken`.
- FDN-CORE-R6: Reconciliation of a closed or expired session fails closed and does not silently create a replacement under the same requestId.
- FDN-CORE-R7: A duplicate stable request without explicit reconcile returns a conflict describing the existing session identity/state and no secret.
- FDN-CORE-R8: Existing account/device/client revocation and possession checks remain required before creation or reconciliation.

## 5. Current state

Canonical Core baseline: `e23abe4e7a616b3f7184cb14d9e7c7904888e0ba`.  
`RemoteComputerSessionCreateRequest` currently contains only `client_id` and `client_token`; session insertion always generates a new UUID and mobile token.

## 6. Target state

Stable requests are persisted on `remote_computer_sessions.request_id` with an account-scoped partial unique index. Legacy rows remain valid with NULL request IDs. Explicit reconciliation authenticates the paired client again, checks device/client/request binding and live session state, rotates `mobile_token_hash`, and returns the same session.

## 7. Architecture and ownership boundaries

The existing Platform Core Remote Computer worker remains the session authority. No new device registry, account store, or parallel Remote Computer service is introduced.

## 8. Interfaces / contracts / schemas / data flow

Request extension:

```json
{
  "clientId": "existing-client-id",
  "clientToken": "existing-possession-secret",
  "requestId": "stable-request-id",
  "reconcile": false
}
```

Legacy clients may omit `requestId` and `reconcile`. Reconciliation requires `requestId`.

Migration adds nullable `request_id TEXT` and a unique index on `(user_id, request_id)` where request_id is not null.

## 9. Constraints and non-functional requirements

All executable verification runs in GitHub Actions. Secrets are never returned in conflict/error responses or persisted in plaintext. The request ID is not an authorization credential.

## 10. Failure modes and edge cases

- malformed requestId -> 400;
- reconcile without requestId -> 400;
- requestId already bound to another device/client -> 409;
- matching live session + duplicate create without reconcile -> 409, no secret;
- matching live session + reconcile -> same sessionId, rotated mobile token;
- matching closed/expired session + reconcile -> 409, no replacement;
- revoked client/device -> existing 403/404 behavior remains authoritative.

## 11. Implementation strategy

1. Add migration for request identity.
2. Extend request/row types and session-create handler.
3. Add schema invariants/tests.
4. Run the repository's Actions workflow on the exact PR head.
5. Only after exact-head green evidence, merge and re-read canonical main.

## 12. Verification / test strategy

Actions must at minimum compile/test the Worker crate and assert migration invariants. Contract tests must cover validation, duplicate conflict, binding conflict, live reconcile credential rotation, expired/closed refusal, and legacy request compatibility. Missing integration runner is blocked rather than pass.

## 13. Acceptance criteria / Definition of Done

- FDN-CORE-AC1: same requestId cannot create two sessions.
- FDN-CORE-AC2: reconcile returns the original sessionId with a newly issued session credential after a simulated lost create response.
- FDN-CORE-AC3: requestId payload drift across device/client is rejected.
- FDN-CORE-AC4: closed/expired stable requests are tombstoned against silent recreation.
- FDN-CORE-AC5: legacy requests without requestId remain accepted under prior semantics.
- FDN-CORE-AC6: exact-head GitHub Actions are green for the applicable Core checks.

## 14. Release / migration / rollback

The schema change is additive. Rollback may stop sending stable request IDs but must not remove the unique index/column while deployed clients rely on reconciliation. Existing NULL rows are unchanged.

## 15. Observability / evidence

Evidence records repository, exact SHA, PR, workflow/run/job IDs, command, result, skips, and artifacts. This spec does not mark any FDN-A01–A16 item passed by itself.

## 16. References / provenance

- Fabushi Desktop `docs/specs/fabushi-same-account-device-network.md` at merge SHA `9fe57d7e23bc11d9a05e08519becc3761accf22e`.
- Core `worker_api/remote_computer.rs` at `e23abe4e7a616b3f7184cb14d9e7c7904888e0ba`.
- Core `AGENTS.md` and `docs/specs/spec-first-ai-development.md`.

## 17. Spec compliance record

| Requirement / AC | Status | Evidence / reason |
| --- | --- | --- |
| FDN-CORE-R1..R8 | pending | implementation and exact-head Actions required |
| FDN-CORE-AC1..AC6 | pending | implementation and exact-head Actions required |
