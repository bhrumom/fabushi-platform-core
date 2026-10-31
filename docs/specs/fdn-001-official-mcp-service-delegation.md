# FDN-001 Official MCP Service Delegation

Status: implementation
Owner: Fabushi Platform Core / account identity issuer
Parent authority: bhrumom/fabushi-desktop/docs/specs/fabushi-same-account-device-network.md (FDN-001)

## Goal

Allow the canonical Fabushi official device-control MCP to call Platform Core on behalf of an explicitly authorized Fabushi account session without storing or forwarding the user's ordinary Fabushi login access/refresh credentials.

This is a delegated service credential inside the existing Fabushi account issuer. It is not a second account system and it does not itself grant control of any device.

## Security model

- The bootstrap request must use a live session-backed Fabushi account access token.
- Only the fixed service id `official-device-mcp` is accepted by this FDN surface.
- The delegated JWT has issuer `https://api.ombhrum.com`, audience `service:official-device-mcp`, subject equal to the Fabushi user id, `tokenUse=service`, the parent account session id, a delegation id, the external MCP OAuth client id hash, the exact MCP resource, and a bounded subset of `devices.read` / `devices.control`.
- Access tokens live for at most 5 minutes.
- Renewal uses a separate opaque rotating service refresh credential. Core persists only its SHA-256 hash.
- Every refresh rechecks the parent Fabushi session. Logout/session revocation therefore fences future service-token renewal even if Backend still holds an old service refresh secret.
- Refresh-token reuse revokes the service delegation.
- The service token is accepted only by explicit Remote Computer API surfaces that opt into service delegation. Existing account, Mini App, provider-MCP, commerce, wallet and marketplace APIs keep their existing auth contracts.
- `devices.read` only permits account-scoped device inventory/status reads.
- `devices.control` does not imply target execution permission. Remote Computer client/session/grant creation, target consent/activation, operation journal checks and Host-side tool permission remain separate mandatory gates.
- Same email, device label, node id or network reachability never establishes subject equivalence.

## API

### Bootstrap

`POST /v1/auth/service-token`

Authenticated with the ordinary Fabushi access token for a live account session.

Body:
- `serviceId = "official-device-mcp"`
- `externalClientId`: bounded MCP OAuth client identifier
- `resource`: exact public official MCP resource URI
- `scopes`: nonempty subset of `devices.read`, `devices.control`

Returns:
- `accessToken`
- `refreshToken`
- `tokenType=Bearer`
- `expiresIn=300`
- `delegationId`

### Refresh

`POST /v1/auth/service-token/refresh`

Body:
- `delegationId`
- `refreshToken`
- `externalClientId`
- `resource`

Rotates the refresh credential atomically. Old refresh reuse revokes the delegation.

### Revoke

`POST /v1/auth/service-token/revoke`

Authenticated by a currently valid delegated service access token. Revokes the delegation and all future refreshes.

### Introspect

`POST /v1/auth/service-token/introspect`

Authenticated by delegated service access token. Returns active subject, service id, scopes, delegation id and parent-session-bound status; never returns credentials.

## Persistence

ACCOUNT_DB migration 0007 adds `account_service_delegations` and `account_service_refresh_tokens`.
Store:
- delegation id
- parent account session id
- user id
- service id
- external client id SHA-256
- resource
- scopes JSON
- current refresh hash
- timestamps/revocation
- rotating refresh hashes and generation

Never store ordinary Fabushi access/refresh tokens or service refresh plaintext.

## Verification

GitHub Actions only:
- shared claim serialization tests
- account schema checks
- token audience/scope/client/resource validation
- refresh rotation/reuse/revocation
- parent-session revocation fencing
- service token cannot authenticate Mini App or ordinary account surfaces
- explicit Remote Computer inventory service-auth contract
- exact-head wasm compile

## Acceptance

- SD-A1: exact-head Core Actions prove claims, migration and Worker compile/tests.
- SD-A2: ordinary Fabushi login token is used only for bootstrap and is not persisted in service delegation state.
- SD-A3: service refresh rotation is independent from login refresh and old-token reuse revokes only the service delegation.
- SD-A4: parent session revocation prevents renewal.
- SD-A5: service JWT validates exact issuer/audience/resource/client/scopes and is rejected from unrelated APIs.
- SD-A6: official MCP Backend can persist only the delegated service credential and call Core inventory without retaining the user's ordinary login credential.

Real device control remains pending until service delegation is composed with Remote Computer client/session/grants, FDN transport and target Host authorization.
