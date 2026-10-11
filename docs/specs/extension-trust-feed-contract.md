# Fabushi Extension Trust Feed Contract v1

Status: active implementation slice  
Parent: Desktop FPM-001 / Unified Extension Contract  
Owner: `native/mahayana-messaging` shared contract crate  
Backend producer: `bhrumom/fabushi-backend` `fabushi.extension-trust-feed.v1`  
Date: 2026-10-11

## Purpose

Define the single platform-neutral parser and verifier for Fabushi Marketplace signing trust. Desktop, iOS and Android native owners consume this shared contract instead of independently interpreting Marketplace trust JSON.

This contract verifies source/integrity and freshness only. It does not grant Host permissions or replace platform-specific install/runtime policy.

## Wire envelope

The server response body contains `trust`, whose envelope is:

- `protocol = "fabushi.extension-trust-feed.v1"`
- `algorithm = "Ed25519"`
- `keyId`
- positive `trustEpoch`
- `publicKey`: base64 SPKI DER Ed25519 public key
- `publicKeyFingerprint`: lower-case SHA-256 hex of those exact SPKI DER bytes
- `issuedAtMs`
- `expiresAtMs`
- `payload`: exact UTF-8 canonical JSON bytes signed by the server
- `signature`: base64 Ed25519 signature of the exact `payload` bytes

Unknown protocol, algorithm, malformed base64, malformed SPKI, invalid fingerprint or unknown security-state values fail closed.

## Signed payload

The exact JSON payload contains:

- protocol;
- issued/expires timestamps equal to the envelope;
- signer algorithm, key ID, trust epoch and fingerprint equal to the envelope;
- complete trust-key list.

Each key contains:
- algorithm;
- keyId;
- positive trustEpoch;
- publicKey;
- publicKeyFingerprint;
- status `active` or `revoked`;
- createdAtMs;
- nullable revokedAtMs;
- revocationReason.

The envelope signer must appear in the signed key list as an `active` Ed25519 key with byte-for-byte identical public key and fingerprint.

## Verification decision

`verify(now_ms)` succeeds only when all of the following hold:

1. envelope protocol and algorithm are known;
2. SPKI public key parses as Ed25519;
3. SHA-256 fingerprint matches;
4. Ed25519 verifies over exact UTF-8 payload bytes;
5. payload deserializes with known security enums only;
6. payload and envelope timestamps/signer identity are identical;
7. `expiresAtMs > issuedAtMs`;
8. `issuedAtMs <= now_ms + 5 minutes`;
9. `now_ms <= expiresAtMs`;
10. the signer is present and active in the signed trust-key list with matching key material.

Any failure returns a typed fail-closed error.

## Offline/stale policy

An expired or otherwise unverifiable trust feed MUST NOT authorize:
- a new Marketplace installation;
- an extension update;
- a newly downloaded artifact;
- restoration of a previously revoked signing key.

Already installed extension execution is not authorized by this contract and remains governed by the platform Host/offline policy. A caller can distinguish `Expired` from cryptographic invalidity for UI/recovery, but both are authorization failures.

## Revocation lookup

After successful feed verification, callers may query a specific `(keyId, trustEpoch)`.

A release signature is trusted only if:
- its key exists in the verified feed;
- key status is `active`;
- algorithm, public key and fingerprint match the release signature envelope.

A revoked or absent key fails closed.

## Cross-platform boundary

This module owns only:
- wire structs;
- strict deserialization;
- cryptographic feed verification;
- freshness decision;
- key-status lookup / release-signing-key admission.

It does not own:
- HTTP fetching/caching;
- account identity;
- Marketplace registry persistence;
- installation state;
- Host capability authorization;
- Mini App sandboxing;
- MCP execution.

Those remain in their existing platform owners.

## Verification

GitHub Actions only.

Required tests:
- valid Backend-shaped vector passes;
- unknown protocol/algorithm/status fails;
- malformed SPKI/base64/fingerprint fails;
- payload/signature tampering fails;
- payload/envelope signer or timestamp mismatch fails;
- signer absent/revoked fails;
- future-dated feed fails;
- expired feed fails;
- active lookup succeeds;
- revoked/absent release-signing key is rejected;
- trust verification never maps to Host permission.

Definition of Done for this slice is exact-HEAD `cargo test --locked` and `cargo check --locked` success in GitHub Actions after lockfile reconciliation.
