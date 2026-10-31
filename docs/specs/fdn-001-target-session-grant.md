# FDN-001 target-authorized official MCP session grant

Status: implementation
Owner: Fabushi Platform Core / Remote Computer

## Purpose

Bootstrap an official-device-MCP controller client without exposing the target
deviceSecret or allowing the controller service token to invoke target-only
authorization.

## API

`POST /v1/computers/:device_id/sessions/grant`

Authentication is the ordinary Fabushi account access token for the target
device account. The request must also contain the registered target
`deviceSecret`.

Body:
- `deviceSecret`
- `connectionId`: official MCP service connection to bind
- `delegationId`: active service delegation to bind
- `requestId`: stable target grant request identity
- `label`: bounded controller label

Core must independently verify in ACCOUNT_DB that the named delegation belongs
to the same user, service id is `official-device-mcp`, connection/delegation
match, parent account session is active, delegation is unrevoked/unexpired, and
scopes include `devices.control`.

A valid grant creates or reconciles one Remote Computer client bound immutably
to that service connection/delegation and returns `clientId/clientToken` to
the target Host. The Host may return that controller credential only over the
authenticated bounded private transport. It never returns deviceSecret.

The same requestId is idempotent and recovers the same service-bound client
credential. A different grant request may not silently replace an already live
service-bound client.

The delegated service access token itself is deliberately not accepted by this
route.

## Security invariants

- account + deviceSecret remain target authority.
- service connection/delegation are looked up by Core, never trusted from Host.
- clientToken is a distinct credential and is never derived in a way that
  reveals deviceSecret.
- service binding is immutable and account/device fenced.
- revoke of the service delegation or Remote Computer client prevents new
  sessions/operations through the existing controller checks.
- request replay with a different binding fails closed.

GitHub Actions is the only build/test authority.
