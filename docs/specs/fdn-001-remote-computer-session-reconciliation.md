# FDN-001 Remote Computer mobile session reconciliation — Core adoption

Status: active  
Owner: Platform Core Remote Computer worker  
Last updated: 2026-10-14

## Context

Fabushi Desktop already accepted and implemented the Remote Computer mobile lost-response contract in PR #60, merged as `92fcc2b0f64b6b8f8f8829305b30874b558d6b35`. Platform Core is the canonical shared Worker owner, but Core main `e23abe4e7a616b3f7184cb14d9e7c7904888e0ba` still carried the exact pre-PR-#60 `remote_computer.rs` blob `931730bff5fb6ee484809233fddc614c5c29b485`.

This change ports that already-reviewed Desktop contract into the canonical Core owner rather than inventing a second reconciliation design.

## Authority

The detailed behavioral contract remains Desktop `docs/specs/remote-computer-mobile-session-reconciliation.md` at `92fcc2b0f64b6b8f8f8829305b30874b558d6b35`, requirements RCS-01 through RCS-10.

Source provenance:

- migration source: Desktop commit `8257b509f3a008541e438db12cf9ac17bf2ae3c8`;
- production create/reconcile source: Desktop commit `f6bc72a8e7d27d2741d12a2c3547d8571205e0ba`;
- router source: Desktop commit `98bb204d626823139b9636d3fc2ca737d0133a87`;
- contract fixture source: Desktop commits `216ba64f847ef0a05626e90300847bb2b1da23a3` and `1d8bbbfe2d8e7a6cce12c1082ef9824a2608685e`.

## Core requirements

- Preserve the exact RCS request identity, request hash and client-token-generation fencing semantics.
- Persist no raw mobile token, paired-client token, or device secret in D1.
- Require stable bounded `requestId` on new mobile session creation.
- Duplicate creates for the same account/device/client/request identity converge on the same session and deterministic session-scoped mobile token.
- Expose the dedicated `POST /v1/computers/:device_id/sessions/reconcile` route.
- Reconcile re-authenticates the current paired client and rejects credential rotation, revocation, cross-account/device/client mismatch, closed sessions and expired sessions.
- Keep activation, signaling, transport, grant and close semantics unchanged.
- Do not create a second account, device registry, session authority or remote-control product.

## Data model

Add migration `0027_remote_computer_mobile_session_reconcile.sql` with nullable legacy-compatible columns:

- `mobile_request_id`
- `mobile_request_hash`
- `client_token_hash_at_create`

and a partial unique index on `(user_id, device_id, client_id, mobile_request_id)`.

The migration number intentionally matches the already-shipped Desktop mirror provenance. A numeric gap in Core does not create a competing schema; migration runners order committed migration files lexicographically and this PR does not import unrelated Desktop migration 0026.

## Verification

All executable validation runs in GitHub Actions. The exact PR head must pass:

- source formatting for files changed by this slice;
- host schema/source-contract tests;
- wasm Worker compilation;
- existing Core broker and extension regressions.

After merge, the same FDN workflow must run on canonical Core `main`; PR-head success is predecessor evidence only.

## Acceptance

This Core slice is complete only when:

1. migration and production source match the accepted Desktop RCS semantics;
2. the reconcile route is wired in the current Core router without overwriting later Core work;
3. exact-head Actions pass;
4. the PR is merged and the merge/main SHA passes the post-merge workflow.

This is only a Phase-1 FDN contract slice. It does not mark FDN-A01 through FDN-A16 complete and does not claim tsnet, Headscale, DERP, reverse WebSocket fallback, official MCP device calls, native device tools or packaged release acceptance.
