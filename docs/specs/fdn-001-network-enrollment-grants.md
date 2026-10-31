# FDN-001 Remote Computer network enrollment grants

Status: implementation contract
Owner: Fabushi Platform Core / Remote Computer
Parent authority: bhrumom/fabushi-desktop/docs/specs/fabushi-same-account-device-network.md (FDN-001)
Predecessors:
- fdn-001-network-node-binding.md
- migration 0031 remote_computer_network_bindings
- existing account + deviceSecret Remote Computer authority

## 1. Goal

Create a short-lived, one-time Core grant that lets an already authenticated
Fabushi Remote Computer request private-network enrollment without receiving
Headscale administrative authority and without trusting a device-supplied
account, cell, node ID, IP address, or network generation.

This grant is only an authorization handoff between the target device and the
dedicated Backend network-enrollment authority. It is not a Tailscale auth key
and it is never sufficient to execute a Remote Computer tool.

## 2. Authority boundaries

- Fabushi account identity remains the product identity authority.
- The target device may issue or revoke its own enrollment grant only with the
  existing account credential plus matching deviceSecret.
- Only the dedicated Core network authority credential
  `FDN_NETWORK_AUTHORITY_TOKEN` may redeem a grant.
- Official MCP service/controller credentials, clientToken, mobileToken, and
  network-node self-report cannot issue or redeem a target enrollment grant.
- Backend/Headscale remains responsible for minting the actual short-lived
  single-use tsnet enrollment key after Core redemption.
- A redeemed grant does not create a Core network binding. Backend must still
  prove the registered network node and call the existing network binding
  authority route.

## 3. Cell and generation authority

The device does not choose `cellId` or `generation`.

Core derives one stable opaque FDN cell identifier from the canonical Fabushi
`user_id` with a domain-separated SHA-256 digest. The cell identifier must
not contain the raw account ID.

The next enrollment generation is:
- 1 when the account/device has never had a Core network binding;
- otherwise exactly the current stored binding generation + 1.

A grant becomes stale if, before redemption, a binding already reaches or
passes its generation.

## 4. Persistence

Migration 0033 adds `remote_computer_network_enrollment_grants` with:

- grant_id (opaque primary key)
- user_id + device_id
- cell_id
- generation
- secret_hash (domain-separated digest; raw secret never persisted)
- created_at
- expires_at
- redeemed_at nullable
- revoked_at nullable

Only one unexpired, unredeemed, non-revoked grant may remain usable for a
device. Issuing a replacement revokes prior live grants for that account/device.

The lifetime is at most five minutes. Expired/redeemed/revoked grants remain
only as bounded audit state and cannot be replayed.

## 5. Interfaces

Target issue:
- POST `/v1/computers/:device_id/network-enrollment-grants`
- ordinary Fabushi account auth required
- body: `{ deviceSecret, requestId }`
- response: `{ grantId, grantSecret, cellId, generation, expiresAt, requestId }`
- requestId is a stable bounded idempotency identity
- retrying the same requestId reconciles the same still-live grant and returns
  the same recoverable secret; a different requestId replaces/revokes an older
  live grant.

Target revoke:
- POST `/v1/computers/:device_id/network-enrollment-grants/:grant_id/revoke`
- ordinary Fabushi account auth + matching deviceSecret
- body: `{ deviceSecret }`
- idempotently revokes only that account/device grant.

Internal redeem:
- POST `/v1/internal/computers/network-enrollment-grants/redeem`
- requires `X-Fabushi-Network-Authority`
- body: `{ grantId, grantSecret }`
- atomically marks one live grant redeemed and returns only
  `{ userId, deviceId, cellId, generation, expiresAt }`
- no raw deviceSecret, account token, grant secret/hash, network proof or
  Headscale credential is returned.

## 6. Secret recovery and idempotency

The raw enrollment grant secret must not be stored. To support safe target
retry of the same requestId, Core derives a recoverable grant secret from the
target deviceSecret, requestId and grantId using a network-enrollment-specific
domain-separated SHA-256 construction, then stores only its domain-separated
hash.

Changing deviceSecret/requestId/grantId changes the recovered secret. A stored
hash mismatch fails closed.

## 7. Failure and replay rules

Issue/revoke/redeem fail closed for:
- invalid or revoked account/device
- mismatched deviceSecret
- invalid/stale request identity
- expired grant
- revoked grant
- already redeemed grant
- secret mismatch
- generation already reached by a current binding
- network authority mismatch

A successful redemption is one-time. Network/API timeout after redemption must
be treated as an uncertain enrollment handoff; callers must query/reconcile
product state rather than redeeming or minting a second key blindly.

## 8. Security invariants

- no raw grant secret is persisted;
- no deviceSecret/account access token/clientToken/mobileToken/network private
  key/Headscale key is stored in the grant table;
- renderer and official MCP controller never receive this internal authority;
- cell identity is Core-derived, not device-selected;
- generation remains monotonic with Core network binding authority;
- target enrollment does not imply tool permission or session authorization.

## 9. Verification

GitHub Actions only. Focused source/schema contracts must prove:
- target issue/revoke use account + deviceSecret;
- internal redeem uses only the dedicated network authority;
- raw secrets are absent from migration/storage;
- five-minute expiry;
- stable Core-derived cell;
- generation is current binding + 1;
- same requestId safely reconciles;
- replacement revokes prior live grant;
- redeem is one-time and generation-fenced;
- official MCP service authority cannot issue/redeem.

## 10. Acceptance

- EG-A1: exact-head schema/source Actions pass.
- EG-A2: no device-controlled cell/generation/node authority is accepted.
- EG-A3: grant is account/device bound, <=5 minutes, one-time and replay safe.
- EG-A4: secret separation and credential boundaries are preserved.
- EG-A5: Backend can redeem under network authority and receives only the
  minimal enrollment projection needed to mint a single-use transport key.

This slice is a G02 prerequisite only. It is not G02/G03/G05 acceptance until
Backend redemption, real Headscale key minting, tsnet registration, Core node
binding, cross-account negative tests, revoke/generation propagation and real
tool calls over the required transports are proven.
