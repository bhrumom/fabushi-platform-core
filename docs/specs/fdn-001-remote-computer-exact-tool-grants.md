# FDN-001 Remote Computer exact-tool grants

Status: implementation contract for the next FDN-001 Platform Core slice.

## Problem

Remote Computer already owns device identity, paired clients, control sessions, session-level grants, stable operation request IDs, request digests, reconciliation, and the durable operation journal. The current operation authorization check only proves that a session-level grant exists and is not expired or revoked. That is insufficient for FDN-001 because the official device MCP exposes a dynamic device tool catalog: tool names are exact opaque identifiers, not a trustworthy category hierarchy.

A controller must not gain execution authority merely because it can see a device, owns a session, or knows a tool descriptor. A target device must not accept a stale capability after revoke/regrant.

## Authority and invariants

- Fabushi account identity remains the tenant boundary.
- Remote Computer remains the authoritative device/session owner.
- Existing session grants remain the coarse session authorization owner.
- Exact-tool grants are an additional capability projection owned by the same Remote Computer session; they are not a second permission system.
- Tool names use the same bounded canonical validation as the operation journal.
- A controller cannot grant itself a tool. Grant and revoke mutations require the target device secret and a currently valid desktop actor for the same account/device/session.
- A tool grant is bound to `user_id + device_id + client_id + session_id + tool_name`.
- Each re-grant increments a monotonically increasing `generation`. A stale generation is never executable.
- Tool grant expiry may not outlive the session and must be in the future.
- Revocation is fail-closed for creation and claim of new execution. Already-running operations may still publish their terminal result so the journal does not convert a known side effect into an artificial unknown outcome.
- Cross-account reads or mutations are impossible because every lookup includes the authenticated account and the bound device/session/client tuple.
- A session without an active exact-tool grant is denied even if its coarse session grant is active.
- Tool descriptors, device names, IP addresses, email addresses, or self-reported node IDs never establish authorization.

## Durable state

Add `remote_computer_session_tool_grants` with one current row per `session_id + tool_name`:

- `session_id`
- `user_id`
- `device_id`
- `client_id`
- `tool_name`
- `generation` (starts at 1, increments on every re-grant)
- `granted_at`
- `expires_at`
- `revoked_at`

Add nullable `grant_generation` to the existing operation journal for migration compatibility. Every newly created FDN operation must persist the generation that authorized creation. Legacy rows may remain null but are never used to authorize a new claim under exact-tool enforcement.

Closing a session revokes all tool grants. Re-granting after a revoke increments generation instead of reusing the old authority.

## API

Target-device authenticated mutations:

- `POST /v1/computers/:device_id/sessions/:session_id/tool-grants/grant`
- `POST /v1/computers/:device_id/sessions/:session_id/tool-grants/revoke`

Grant body:

```json
{
  "deviceSecret": "...",
  "clientId": "...",
  "toolName": "browser.click",
  "expiresAt": 1234567890
}
```

Revoke body:

```json
{
  "deviceSecret": "...",
  "clientId": "...",
  "toolName": "browser.click"
}
```

Grant returns the exact tool name, generation, and expiry. Revoke is idempotent and returns the last generation when a row exists.

## Operation journal integration

Operation create performs, in order:

1. authenticate the Fabushi account;
2. validate device/session/client actor and coarse session grant;
3. validate canonical `toolName`;
4. load an active exact-tool grant for that exact name;
5. persist its generation with the request digest and stable request ID.

Operation claim performs target-device actor verification, loads the durable operation, and requires the exact tool grant to still be active at the stored generation before changing `pending -> running`.

Operation result keeps the existing target/device/session/client and journal checks. It may complete a previously claimed operation after tool revoke so the system records the real terminal outcome.

## Required tests

GitHub Actions must prove:

- exact tool A cannot authorize tool B;
- no tool grant means create is rejected;
- controller credentials cannot issue a tool grant;
- grant expiry is bounded by session expiry;
- re-grant increments generation;
- revoke makes create fail;
- revoke between create and claim makes claim fail;
- stale generation cannot claim after revoke/regrant;
- same-account but different device/client/session cannot reuse a grant;
- another account cannot query or mutate the grant or its operations;
- stable requestId/digest conflict and outcome_unknown/terminal monotonicity remain unchanged;
- session close revokes exact-tool grants;
- result publication for an already-running operation remains possible after tool revoke.

All builds, schema validation, and tests run only in GitHub Actions.
