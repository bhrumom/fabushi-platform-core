# Telegram MTProto destroy_auth_key request wire contract

Status: Stage A implementation contract; executable validation deferred.

## Fixed-U provenance

- Upstream: `telegramdesktop/tdesktop@f985723b552b30fe1c17d377d57f44e61a15a8c4`.
- `Telegram/SourceFiles/mtproto/scheme/mtproto.tl@76225965bfc794388db241f3f760db8ab79e84f8` defines:
  - `destroy_auth_key#d1435160 = DestroyAuthKeyRes`.
  - `destroy_auth_key_ok#f660e1d4 = DestroyAuthKeyRes`.
  - `destroy_auth_key_none#0a9f2259 = DestroyAuthKeyRes`.
  - `destroy_auth_key_fail#ea109b13 = DestroyAuthKeyRes`.

## Scope and ownership

This slice adds only the zero-field request serializer to the existing canonical Rust `telegram-protocol` owner and the byte-identical desktop provider copy. It does not decide when an auth key is eligible for destruction, mutate persisted key/session state, close transports, revoke accounts, or synthesize Telegram Desktop's key-age/session lifecycle.

## Requirements

- R1: Export `DESTROY_AUTH_KEY_CONSTRUCTOR = 0xd1435160`.
- R2: Export `build_destroy_auth_key() -> Vec<u8>`.
- R3: The builder emits exactly four little-endian bytes containing only the constructor.
- R4: The builder has no parameters, lookup, network, persistence, scheduler, callback, session, ACK/resend/container, gzip, or updates-difference side effect.
- R5: Auth-key eligibility, remote result handling, local deletion, teardown, reconnect, and historical session/key lifecycle remain fail-closed until represented by an exact existing owner.
- R6: Stage-A contracts are authored but not executed.

## Authored contracts

- exact four-byte constructor layout;
- no payload/trailing request fields;
- public re-export through the existing protocol facade;
- desktop provider source remains byte-identical to canonical source after synchronization.

## Acceptance

Source-level implementation may advance to `implemented / validation_status=not-run / accepted=false`. No build, lint, schema validator, generator, unit/contract/integration/E2E, package, deployment, installation, GitHub Actions, checks, or statuses are executed or queried in Stage A.


## Round 39 source-level compliance readback

Canonical implementation commit: `8eafd0cc7a84ab9af6b402665670b38f3ad72406`. Desktop synchronization commit: `fe0093d767ad2a12e049b4d0a474c0145d3468ab`. Desktop bookkeeping commit: `36d0f218cf5320d292250ec8beea3a641838f072`.

Remote source readback confirms canonical and desktop provider api/lib blobs are byte-identical at `fbca9b0ebb85d0edd5c01ceea9896d0977294b53` / `ed581d8356375e264fabe85a2e5cccbd9b772f21`. `DESTROY_AUTH_KEY_CONSTRUCTOR=0xd1435160` and public `build_destroy_auth_key()` are present and the authored contract asserts exactly four constructor bytes with no payload.

R1-R5 are satisfied at source/ownership/bookkeeping level. R6 executable validation remains intentionally `not-run`, with ledger `accepted=false`. No auth-key destruction eligibility, remote-result application, local key deletion, persistence, teardown/reconnect, arbitrary sent/resend/container history, gzip, or complete difference lifecycle is claimed.

No build, typecheck, lint, schema validator, generator, unit/contract/integration/E2E, package, deployment, installation, GitHub Actions, checks, or statuses were run or queried.
