# FDN-001 Official MCP Remote Computer controller operations

Status: implementation contract
Owner: Fabushi Platform Core / Remote Computer
Parent authority: bhrumom/fabushi-desktop/docs/specs/fabushi-same-account-device-network.md (FDN-001)

## Goal

Allow an already paired and session-bound canonical Fabushi official-device-MCP service controller to use the existing Remote Computer durable operation journal without granting the service credential any target-device authority.

## Invariants

- Controller requests may authenticate with either the existing ordinary Fabushi account credential or a valid official-device-mcp service credential carrying `devices.control`.
- A service controller must use a Remote Computer client whose immutable `service_connection_id` and `service_delegation_id` exactly match the verified service JWT claims on every controller operation request.
- Ordinary account controller requests may use only ordinary clients whose service-binding columns are both null.
- The existing active session, exact client id, mobile token, session expiry, coarse grant, device/account fence, stable requestId, request digest and requestId conflict rules remain unchanged.
- Operation creation additionally requires the existing exact-tool grant and records its generation. No service credential may mint, replace or revoke a target tool grant.
- Target operation claim and result publication remain ordinary Fabushi account + deviceSecret paths and revalidate the exact persisted grant generation before execution.
- Service delegation revoke, parent Fabushi session revoke, service expiry, wrong connection id, wrong delegation id, account switch, or client revoke fails new controller operation authorization closed.
- Query/cancel/outcome_unknown are limited to the same service-bound controller client/session and keep existing monotonic journal state rules. Unknown outcomes are never interpreted as not-executed and never automatically replayed.
- Device secret, client token, mobile token, ordinary login token, service refresh token and network private keys remain separate credentials. This slice does not expose any of them to target tool arguments or result payloads.

## API

Existing controller endpoints are reused compatibly:
- create operation
- query operation
- mark outcome_unknown
- cancel pending operation

No second operation database, service-specific operation table or alternate request identity is introduced.

## Verification

GitHub Actions only. Exact-head contracts must prove:
- ordinary controller behavior remains unchanged;
- service controller with matching connection/delegation + active session/mobile token + exact tool grant may create one operation;
- wrong service connection/delegation is rejected;
- service credential cannot grant tools, claim operations, or publish results;
- grant generation remains persisted and target claim rejects revoked/replaced generations;
- requestId conflict, outcome_unknown and cancellation semantics remain unchanged;
- service revoke/expiry authentication gates every new controller operation request.

## Acceptance

- OP-A1: exact-head Remote Computer and service-auth Actions pass.
- OP-A2: no operation can be created solely from same-account visibility or a service token without pairing/session/mobile-token/exact-tool grant.
- OP-A3: target claim/result authority remains deviceSecret-owned.
- OP-A4: no journal uniqueness, digest, outcome_unknown or terminal monotonicity invariant is weakened.
