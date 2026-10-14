# FDN-001 target network ingress peer projection

Status: implementation contract
Owner: Fabushi Platform Core / Remote Computer network binding owner
Parent: bhrumom/fabushi-desktop/docs/specs/fabushi-same-account-device-network.md (FDN-001)

## Context

The Desktop target-side network worker already accepts bounded tsnet streams only from an exact expected peer. The official MCP controller can obtain the target device network route, but the target Host has no authoritative projection of which controller/gateway node is allowed to initiate that private connection. Accepting every tailnet peer would weaken the existing FDN peer restriction and is not an acceptable cutover.

## Goal

Extend the existing Remote Computer network binding owner with a minimal account/cell-scoped ingress-node projection. A dedicated network authority binds the authenticated Fabushi gateway/controller node for one account and cell. A target device may read only the live ingress peer for its own account and current network cell by presenting the existing Fabushi account credential plus matching deviceSecret.

## Non-goals

- This projection does not authorize a tool, session, or operation.
- It does not replace target network binding, service delegation, exact-tool grants, requestId/digest checks, or Host application-layer authorization.
- It does not mint Headscale keys or start a gateway worker.
- It does not allow renderer, ordinary MCP service credentials, clientToken, or mobileToken to write ingress bindings.
- It does not accept wildcard peers or expose an arbitrary cell snapshot to a target device.

## Requirements

- IN-R1: write/revoke is internal-only and requires the existing constant-time FDN network authority credential.
- IN-R2: one live ingress binding is keyed by immutable Fabushi userId + cellId + serviceId. The initial serviceId is `official-device-mcp`.
- IN-R3: a binding stores bounded nodeId, positive monotonic generation, boundAt, expiresAt, and optional revokedAt. Node IP/name self-report is never trusted from a device.
- IN-R4: a target read authenticates ordinary Fabushi account + matching deviceSecret, resolves that device's current live network binding, then returns ingress only for the same account and cell.
- IN-R5: target read returns no network authority secret, deviceSecret, service credential, Headscale auth key, proof material, or other-account identity.
- IN-R6: revoked/expired ingress or target binding fails closed. Account switch automatically changes the account-scoped lookup.
- IN-R7: rotation must advance ingress generation. Stale generation and cross-account/cell reads fail closed.
- IN-R8: target transport must continue to pass the returned exact nodeId to the existing `stream_listen(expectedPeer,...)`; this projection never permits wildcard listener behavior.

## API

Internal network-authority routes:
- `POST /v1/internal/computers/network-ingress/bind`
  - body: `{ userId, cellId, serviceId, nodeId, generation, expiresAt }`
- `POST /v1/internal/computers/network-ingress/revoke`
  - body: `{ userId, cellId, serviceId, generation }`

Target device route:
- `POST /v1/computers/network-ingress`
  - body: `{ deviceId, deviceSecret, serviceId }`
  - ordinary account auth required
  - response: `{ deviceId, cellId, targetGeneration, serviceId, nodeId, ingressGeneration, expiresAt, active }`

## Failure / recovery

Binding/revoke is idempotent only for the exact current generation and identity. Conflicting node changes require a higher generation. Expiry/revoke makes the peer unreadable immediately at Core. A target listener that already accepted a stream still performs the existing network-binding generation check and target service/session/grant/request checks before native execution.

## Verification

GitHub Actions only. Exact-head tests must prove:
- migration uniqueness/monotonic guards;
- network-authority-only bind/revoke;
- account+deviceSecret target read;
- same-cell restriction;
- cross-account/device rejection;
- revoke/expiry rejection;
- no protected credential in response;
- existing network binding/controller operation tests remain green.

## Acceptance for this slice

This slice is ready to integrate when the exact PR head passes the repository FDN Remote Computer workflow(s). It is only the missing peer-selection contract; it does not claim real direct/DERP/WSS E2E.
