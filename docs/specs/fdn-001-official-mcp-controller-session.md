# FDN-001 Official MCP Remote Computer controller session binding

Status: implementation contract
Owner: Fabushi Platform Core / Remote Computer
Parent authority: bhrumom/fabushi-desktop/docs/specs/fabushi-same-account-device-network.md (FDN-001)

## Goal

Allow the canonical Fabushi official device-control MCP service delegation to become an ordinary Remote Computer controller client only after consuming the existing target-generated one-time pairing code, and to create/reconcile a control session using the resulting client credential.

This does not let a service token execute a device tool. Target activation, coarse session grant, exact-tool grant, operation journal authorization, network routing, and Host-side authorization remain separate mandatory gates.

## Security invariants

- The service credential must pass the existing official-device-mcp issuer/audience/resource/scope/parent-session/delegation checks with `devices.control`.
- Pairing still requires the existing single-use, expiring pairing code generated for a device in the same Fabushi account. Service reachability or `devices.control` alone cannot mint a client.
- A client paired by the service is stored in the existing `remote_computer_clients` authority, not a second controller database.
- Service clients are durably bound to the service JWT `connectionId` and `delegationId`.
- A service-bound client token cannot be used by an ordinary account controller path, by another official MCP connection, or by a refreshed/replaced delegation with a different delegation id.
- An ordinary paired client remains unbound and behaves exactly as before.
- Service session create/reconcile uses the existing stable requestId, request hash, recoverable mobile token, session limits, expiry, and account/device/client fences.
- Service token revocation or parent Fabushi session revocation immediately prevents new service session create/reconcile because service authentication fails closed.
- This slice deliberately does not authorize service credentials for operation create/query/cancel/outcome_unknown. Those remain ordinary-controller-only until a later exact-grant-aware slice.
- Device secret, client token, mobile token, ordinary Fabushi access token, service refresh token, and network keys remain separate credentials and are never returned by inventory or audit surfaces.

## Persistence

Migration 0030 extends `remote_computer_clients` with nullable:
- `service_connection_id`
- `service_delegation_id`

Both are null for ordinary clients. They are both non-null for official MCP service-paired clients. A partial unique index prevents the same official MCP connection from silently creating multiple live authority identities for the same device, and a trigger prevents service binding mutation after creation.

## API behavior

Existing routes are reused:
- `POST /v1/computers/pair`
- `POST /v1/computers/:device_id/sessions`
- `POST /v1/computers/:device_id/sessions/reconcile`

For these three routes only, authentication accepts either:
1. an ordinary Fabushi account token, preserving current behavior; or
2. a valid official MCP service token with `devices.control`.

When the service path is used, the created client row records the exact `connectionId` and `delegationId`. Session create/reconcile additionally require that exact binding.

## Verification

GitHub Actions only:
- ordinary pairing/session/reconcile behavior remains unchanged;
- service token without devices.control is rejected;
- service control token without pairing code cannot create a client;
- service pairing consumes the same single-use pairing code;
- resulting client row binds connectionId + delegationId;
- ordinary account auth cannot use a service-bound client;
- another service connection/delegation cannot use that client;
- matching service binding can create and reconcile the same stable session request;
- parent-session/service revocation fences later create/reconcile;
- operation create remains service-token denied in this slice;
- schema migration and source contract are exact-head checked.

## Acceptance

- CS-A1: exact-head Actions prove migration/source/contract behavior with no regression to existing Remote Computer session reconciliation.
- CS-A2: no service controller exists without explicit target pairing-code consent.
- CS-A3: service-bound client identity cannot cross account, connection, or delegation boundaries.
- CS-A4: stable session request identity/reconciliation semantics are unchanged.
- CS-A5: no operation execution authority is added by this slice.
