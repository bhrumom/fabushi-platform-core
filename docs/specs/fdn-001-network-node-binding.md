# FDN-001 Remote Computer network-node binding projection

Status: implementation contract
Owner: Fabushi Platform Core / Remote Computer
Parent authority: bhrumom/fabushi-desktop/docs/specs/fabushi-same-account-device-network.md (FDN-001)
Source baseline: c2e99921236890f8c76ea7f2c5c3b41ba4e05739

## 1. Context / problem

The canonical Remote Computer device authority currently authenticates a Fabushi account and deviceSecret, but it does not persist a trusted projection that proves which private-network node belongs to that exact account/device generation. A device-supplied nodeId is not proof and must never become a trust root.

## 2. Goal

Add a fail-closed Core projection for a trusted network-enrollment authority to bind, rotate, expire, and revoke the private-network node identity of an existing Remote Computer while preserving account/device/session/operation ownership boundaries.

## 3. Non-goals / out of scope

- This slice does not mint Tailscale/Headscale keys or network private keys.
- It does not expose arbitrary network dial authority to Renderer, MCP, or clients.
- It does not make a network binding sufficient for tool execution.
- It does not deploy Headscale, DERP, reverse-WSS, or prove real routes.
- It does not change the existing operation-journal request identity or terminal-state rules.

## 4. Requirements

- R1: a network binding is keyed by the existing Fabushi account user_id + Remote Computer device_id; device name, IP, email, or self-reported nodeId cannot establish trust.
- R2: only a dedicated internal network-enrollment authority credential may create/rotate/revoke a binding. Ordinary account tokens, official MCP service tokens, deviceSecret, clientToken, and mobileToken cannot write it.
- R3: every live binding contains a bounded cellId, nodeId, proof SHA-256 digest, positive generation, bound_at, expires_at, and nullable revoked_at.
- R4: generation is monotonic. Rotation/re-enrollment must advance generation; stale or replayed generations fail closed.
- R5: one live nodeId cannot be bound to multiple account/device authorities.
- R6: expiry and revoke are independently represented. A revoked or expired binding is never reported as active.
- R7: bind/rebind/revoke events are account/device scoped in an immutable audit table. No raw proof, user access token, Remote Computer secret/token, network private key, or tool argument is persisted.
- R8: an authenticated device may read only its own binding projection by presenting the existing Fabushi account credential plus matching deviceSecret. Cross-account/device reads fail closed.
- R9: the public computer inventory may expose only bounded network readiness metadata (bound/active generation/expiry), not proof material, private keys, or arbitrary network addresses.
- R10: account switch/network revoke is representable immediately by revoking the old account/device binding; later asynchronous ACL cleanup is not treated as application authorization.

## 5. Current state

Remote Computer registration is account + deviceSecret fenced. Sessions, exact-tool grants, and operation journal rows are already user_id/device_id scoped. There is no canonical network node proof/generation/expiry/revoke projection.

## 6. Target state

Core stores a separate network-binding projection owned by Remote Computer. A trusted Backend enrollment authority writes the projection through internal-only endpoints. Desktop/Host can read the currently active generation using its existing account + deviceSecret authority. Other control surfaces receive only coarse readiness.

## 7. Architecture and ownership boundaries

Fabushi account remains product identity authority. Remote Computer remains device/session authority. Backend/coordination is the enrollment authority that proves a network node and writes the projection. Tailscale/Headscale remains transport only. Host still performs application-layer authorization for every operation.

## 8. Interfaces / contracts / schemas / data flow

Migration 0031 adds:
- remote_computer_network_bindings
- remote_computer_network_binding_audit
- monotonic/binding-integrity triggers and live-node uniqueness

Internal authority routes:
- POST /v1/internal/computers/network-bindings/bind
- POST /v1/internal/computers/network-bindings/revoke

Both require X-Fabushi-Network-Authority matching the secret FDN_NETWORK_AUTHORITY_TOKEN in constant time.

Device read route:
- POST /v1/computers/network-binding
- body: { deviceId, deviceSecret }
- requires ordinary Fabushi account auth and matching existing deviceSecret.

The bind request contains userId, deviceId, cellId, nodeId, proofSha256, generation, expiresAt. The proof itself is never persisted.

## 9. Constraints and non-functional requirements

Identifiers and digests are bounded/validated. Expiry must be in the future and bounded to a maximum projection lifetime. Internal authority credentials are never returned or logged by this code. Existing WebRTC/session behavior remains compatible.

## 10. Failure modes and edge cases

Unknown account/device, revoked device, authority-secret mismatch, cross-account deviceId, non-advancing generation, duplicate live nodeId, invalid digest, expired enrollment, and already-revoked generation all fail closed.

## 11. Implementation strategy

1. Add migration 0031 and exported schema constant.
2. Add strict request validators and internal authority authentication.
3. Implement bind/revoke/read handlers in the existing Remote Computer owner.
4. Add routes in the existing worker router.
5. Extend inventory output with coarse active network metadata only.
6. Add schema/source contract tests; use GitHub Actions only.

## 12. Verification / test strategy

GitHub Actions must prove migration constraints and source contracts, existing Remote Computer tests remain green, and the new handlers preserve secret separation and account fences. No local build/test is acceptable evidence.

## 13. Acceptance criteria / Definition of Done

- NB-A1: exact-head Actions pass with migration/source contract checks.
- NB-A2: device self-report without internal authority cannot bind a node.
- NB-A3: stale generation/replay and cross-account binding/read attempts fail closed.
- NB-A4: revoke/expiry make the projection inactive without weakening application authorization.
- NB-A5: no raw proof or protected credential is persisted or exposed.

## 14. Release / migration / rollback

Migration is additive. Rollback disables the new routes/consumer use while retaining rows for audit; no existing Remote Computer schema column is removed.

## 15. Observability / evidence

Evidence is exact-head GitHub Actions run/job output plus migration/source commit SHA. Production route evidence remains pending until Backend coordination and Desktop Host consume this projection.

## 16. References / provenance

- FDN-001 Desktop authority
- migrations 0028 operation journal, 0029 exact tool grants, 0030 service client binding
- existing Remote Computer account/device/session owner

## 17. Spec compliance record

| Requirement / AC | Status | Evidence / reason |
| --- | --- | --- |
| R1-R10 | pending | implementation and exact-head Actions required |
| NB-A1-NB-A5 | pending | implementation and exact-head Actions required |
