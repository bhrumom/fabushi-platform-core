# FDN-001 Official MCP operation-controller binding

Status: implementation contract
Owner: Fabushi Platform Core / Remote Computer
Parent authority: bhrumom/fabushi-desktop/docs/specs/fabushi-same-account-device-network.md (FDN-001)
Source baseline: b5be3e0b6652fc05fe32ba3f0c3b7082922c80da

## 1. Context / problem

Core already issues a short-lived official-device-MCP service delegation and binds a paired Remote Computer client to the exact service connectionId + delegationId. The same service credential can create/reconcile a control session. However the controller-side operation journal endpoints still accept only ordinary Fabushi account tokens. Backend therefore cannot use the canonical operation journal without retaining/reusing a user's ordinary login credential.

## 2. Goal

Permit a valid official-device-MCP devices.control service delegation to create, query, cancel, and mark outcome_unknown for operations only when the referenced paired client is bound to that exact service connection/delegation and the existing session/mobile-token/grant fences pass.

## 3. Non-goals

- This does not let service credentials claim or complete target-side operations.
- This does not grant a tool, activate a session, mint pairing codes, or bypass target consent.
- This does not expose deviceSecret or network credentials.
- This does not automatically retry outcome_unknown operations.
- This does not replace Host-side secondary authorization.

## 4. Requirements

- SO-A1: controller operation endpoints accept either a normal Fabushi account actor or an official-device-MCP service actor with devices.control.
- SO-A2: service actors must match the paired client's immutable service_connection_id and service_delegation_id. A different official MCP connection/delegation under the same Fabushi account is rejected.
- SO-A3: existing mobileToken/session actor checks and exact-tool grant generation remain mandatory.
- SO-A4: requestId, request digest, unique journal identity, outcome_unknown and terminal monotonicity are unchanged.
- SO-A5: target-side claim/result endpoints keep their existing target authority and are not opened to service credentials.
- SO-A6: ordinary account actors may operate only non-service-bound clients; they cannot take over service-bound clients.
- SO-A7: service token revoke/expiry/account-session fencing continues to be enforced by the existing service-auth verifier before any journal operation.

## 5. Interfaces

Affected controller endpoints only:
- POST /v1/computers/:device_id/control/sessions/:session_id/operations
- POST /v1/computers/:device_id/control/sessions/:session_id/operations/:request_id/query
- POST /v1/computers/:device_id/control/sessions/:session_id/operations/:request_id/cancel
- POST /v1/computers/:device_id/control/sessions/:session_id/operations/:request_id/outcome-unknown

The request/response schemas remain unchanged. The Authorization bearer may now be the existing official-device-MCP delegated service access token when its binding matches.

## 6. Verification

GitHub Actions only. Contract tests must prove:
- all four controller handlers use remote_controller_account;
- all four enforce remote_controller_client_binding_matches before operation authorization;
- claim/result handlers do not use the service-controller path;
- operation request identity/digest/grant code remains present;
- wasm worker compiles.

## 7. Acceptance / compliance

| Requirement | Status | Evidence |
| --- | --- | --- |
| SO-A1..SO-A7 | pending | exact-head Actions required |

Production official MCP -> Backend -> Core -> Host E2E remains pending and is not satisfied by this slice.
