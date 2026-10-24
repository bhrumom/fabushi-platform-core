# Fabushi native cloud services

Status: active; implementation and production acceptance incomplete
Date: 2026-10-24
Owner: existing Mahayana Platform Worker; Desktop Host service consumers; Backend public gateway

## Authority and goal
User explicitly requires removing Cursor service dependencies across Fabushi, retaining cloud automation, cloud agents and cloud file synchronization as Fabushi-owned capabilities. This supersedes historical vendor-service parity. The service state owner remains the existing Rust Worker and its PLATFORM_DB, account identity remains ACCOUNT_DB. Backend owns public gateway and deployment; Desktop owns product execution and local materialization. No parallel service/database/agent runtime is introduced.

## Current sources and ownership
Core baseline c7d0e24; Desktop baseline 7394f6dc. Core Worker has live-session authentication, durable listeners, device execution/operation journal and account infrastructure. Desktop's Worker copy includes account_agent_store, absent from Core main: source reconciliation is required before claiming canonical deployment supports file synchronization. Backend Node AccountSyncStore also has agent/file data; it is not automatically the public /v1 authority because the gateway delegates /v1 to the Rust Worker. Do not substitute source existence for production proof.

## Required service migrations
NS-R1: native automation definitions, schedules (timezone/DST/clock), trigger ingress, account/device-scoped delivery, idempotent completion, durable restart reconciliation and ACK semantics.
NS-R2: native cloud Agent launch/status/transcript/artifacts/cancel/archive/environments use existing worker/device execution owner and grants. No Cursor session, RPC or cloud dashboard required.
NS-R3: native agent/file synchronization reuses existing account state/CAS/conflict owners; preserve encryption, multipart/large-file, access, deletion, sharing and offline/restart semantics. Reconcile source owners before deployment.
NS-R4: migrate remaining model/media/search/setup/marketplace/notifications/feedback/privacy/experiments/update/telemetry consumers to existing native or local owners, removing foreign endpoint defaults and vendor credentials. Preserve feature behavior; silent disablement, placeholder success and renamed unsupported RPCs are incomplete.
NS-R5: account/session revocation, origin/issuer binding, tenant/device/grant isolation and late response generation fences apply to all operations. Never migrate foreign tokens into native auth.
NS-R6: all executable verification in GitHub Actions. Native real-service/package/network acceptance on current revisions is mandatory for complete removal, separately from schema/mocks/wasm compilation.

## First implementation unit: durable automation run journal (protocol fabushi.automation.v1)
This establishes the previously missing completion contract. It does not complete scheduler/cloud Agent/file-service migration.
- POST /v1/automation-runs creates an account-owned event using runUuid, sandAgentId (legacy field name only), automationId, timestampMs, optional definitionRevision/scheduledForMs/event. Explicit owner manual enqueue only; external connector ingress remains under its existing trusted adapter.
- POST /v1/automation-events/poll takes ackRunUuids (max 100), returns unacknowledged events and bounded nextPollAfterMs. Acknowledgement mutates only account-owned completed rows. Unknown/not-completed ACKs fail atomically, never delete pending runs.
- POST /v1/automation-runs/complete takes runUuid,status (succeeded|failed),errorMessage. First completion persists atomically; identical retry returns the same receipt (200 JSON). Conflicting completion returns 409 without mutation. Missing or foreign run returns indistinguishable 404. Revoked/expired session returns 401. This protocol never returns 204.
- GET /v1/automation-runs/:run_uuid returns current account-scoped event, completion and ACK state for restart reconciliation. No automatic replay from outcome-unknown; Desktop must reconcile durable execution history before deciding to execute.
- UUID identity is account-scoped, request fields bounded and strict. Same runUuid with identical canonical event is idempotent; different event conflicts. Receipt is stable canonical completion digest plus stored completion time, not a client assertion of success.
- Durable PLATFORM_DB tables keep terminal tombstones; poll ACK does not erase idempotency history. Delivery is at-least-once; account-scoped polling is not a device lease. Before multi-device execution cutover add device/grant lease arbitration. Do not claim exactly-once side effects.

## Architecture and plan
Add one small automation domain module under the existing Worker and one additive migration; use authenticated_session_account, canonical JSON digest and D1 transactions/batches/CAS. Keep domain validation/transition functions testable on host, exercise SQL on actual SQLite in Actions, compile actual wasm routes. Desktop retains its existing Automations execution owner. Native transport must use explicit native credentials and paths; never generic Cursor/Sand token access. Deploy migration and service before enabling product cutover. Then close schedule/lease/recovery and remaining NS-R1..R4 units.

## Failures and observability
All failure responses retain work and expose typed error codes without tokens/event contents in logs. D1 failures are non-success. Duplicate/response-loss/restart tests must read canonical receipts. Cross-account identifiers do not disclose existence. Row limits and bounded event size protect storage; retention/quotas before production release. No pairing mutation is needed by this journal unit.

## Verification and acceptance
AC1: typed validation and canonical identical/conflict decisions have positive/negative host tests.
AC2: real SQLite proves account-scoped uniqueness, complete-once receipt, conflicting retries, invalid ACK atomicity, persistence after reopen and cross-account isolation.
AC3: actual Worker wasm routes compile and authenticate live sessions; HTTP integration checks exact 200/400/401/404/409 semantics.
AC4: Desktop native call chain and durable recovery use the new contract and pass fault-injection/multi-device tests before cutover.
AC5: NS-R1..R6 all have current service/package evidence; zero automatic Cursor service traffic. Full completion requires all ACs, not just the journal.

## Release / migration / rollback
Additive migration, no destructive conversion of account/session/agent/file records. Normal reviewed exact-head pipeline only; no production deployment in the initial journal PR. Do not enable Desktop until applicable service acceptance. Rollback disables native cutover and preserves journal/tombstones; never falls back to Cursor. Existing signed release processes apply.

## Compliance
All AC1..AC5 pending. Initial journal implementation is only a scoped unit; no service migration or release completion claimed. Desktop PR #172 changes endpoint configuration only; PR #171 handles unconfirmed completion ACK.

## Native account usage integration unit
Desktop replaces the foreign usage dashboard with the existing /v1/ai/usage projection and in-app Usage surface. That read route must use the same active account-session gate as the new journal, return 401 for revoked/mismatched sessions and preserve the existing authoritative token-budget calculation. No changes to model-gateway reserve/capture privileges or budget arithmetic. Actual wasm/D1 tests seed different account budgets, verify separate values, then exercise real logout and refusal of the old account token. This is additional scoped NS-R4/NS-R5 evidence, not cloud service completeness.
