# Fabushi Unified Extension Contract

Status: active implementation slice  
Parent: Desktop FPM-001  
Owner: `native/mahayana-messaging` shared contract crate  
Date: 2026-10-11

## Purpose

Define the platform-neutral identity, origin, trust and compatibility vocabulary shared by Desktop, iOS, Android and Server. This is a data contract only; it is not a Marketplace service, installer, runtime, permission engine or account database.

## Stable v1 vocabulary

Extension kinds:
- `mcp-connector`
- `skill`
- `web-miniapp`

Origins:
- `public`
- `team`
- `private-created`

Trust domains:
- `catalog-authorized`
- `team-authorized`
- `private-local-author`

Release channels:
- `stable`
- `preview`

Platforms:
- `desktop`
- `ios`
- `android`
- `web`
- `cli`
- `mobile`

Stable identity is `<namespace>/<name>:<kind>`. Namespace and name are lower-case ASCII identifiers and must not be silently normalized from invalid input.

Trust and authorization are separate: a trusted catalog source never implies Host permission.

Unknown enum variants must fail deserialization for v1 rather than silently acquiring capabilities.

## Compatibility

A release declares:
- semantic version string;
- release channel;
- platform set;
- optional minimum Host API;
- requested capability names;
- revocation state.

A client may display an unsupported release but must not expose an install action unless its platform and Host API are compatible.

## Cross-platform requirement

All native clients and the Backend Registry must use equivalent serialized values. Golden JSON vectors and exact-head GitHub Actions are the compatibility oracle.

## Non-goals

This slice does not implement registry persistence, publisher namespaces, signatures, downloads, install state, OAuth, MCP execution, Mini App sandboxing or Skills execution.
