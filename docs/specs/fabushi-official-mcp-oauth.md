# Fabushi official MCP marketplace — Specification
Status: active
Owner: Fabushi Platform Core / existing account connection broker
Last updated: 2026-10-09

## 1. Context / problem
The user requests a Fabushi-owned official marketplace listing common provider-operated MCP servers, especially Google and GitHub. Core baseline is 7e24aa179a44a206eceb722e650a1db6d5bb1f59. Desktop integration is PR #49 in bhrumom/fabushi-desktop. The current recovered MCP catalog, effective-install lookup and HTTP MCP execution depend on Cursor Dashboard. Fabushi account credentials are correctly refused for that foreign backend; the UI's all-or-nothing loading hides even available catalog entries.

## 2. Goal
Extend the existing Plugins surface and Host/Coordinator MCP relay with a Fabushi-maintained catalog, durable account-scoped installation, provider credential isolation and real remote MCP discovery/execution. Distinguish listed, installed, authorized and connected.

## 3. Non-goals
Do not rebuild Google/GitHub APIs, impersonate provider publishers, weaken account-token origin checks, replace Coordinator/Host/Runner, or invent provider tools. Do not claim OAuth/live production acceptance before configuration and evidence.

## 4. Requirements
- FMCP-001: list GitHub and Google Gmail, Drive, Docs, Sheets, Slides, Calendar, Chat and People using exact provider-documented HTTPS endpoints. Google listings disclose Developer Preview, Cloud project/API enablement and OAuth prerequisites.
- FMCP-002: distinguish Fabushi directory curation from upstream provider ownership. Stable namespaced IDs, descriptions and official provenance are bundled and available without foreign Dashboard access.
- FMCP-003: installation/update/removal and account credentials persist in the existing encrypted storage mechanism, in a dedicated file isolated from Box secret export. Fabushi account identity scopes every operation. No third-party service receives a Fabushi session token.
- FMCP-004: installed connectors participate in shipping MCP server/tool inventory and Coordinator/Host routed tools. Implement initialize, notifications/initialized, tools/list pagination and tools/call with Streamable HTTP JSON and SSE responses. Requests are bounded, redirects refused, tools validated and errors redacted; uncertain writes are never automatically retried.
- FMCP-005: install without authorization remains needsAuth. Explicit provider token setup is supported as an interim route (GitHub PAT / Google OAuth access token), stored only in encrypted native storage and never returned by catalog/list/setup reads. Production one-click OAuth is required: reuse the existing Fabushi account identity and platform Worker, register or reuse suitable owner-controlled clients, and provide state-bound provider consent, token expiry/refresh and revocation. Token setup is not production acceptance.
- FMCP-006: failures in legacy installed/custom connectors must not hide the official directory or its installed connectors. Show a partial-load warning; do not silently report legacy lookup success.
- FMCP-007: preserve custom/team/private-skill flows and existing permission/tool-disable gates. Account switches, uninstall or credential changes fence in-flight results. Native error/tool output must not reveal provider credentials.
- FMCP-008: all executable verification runs in GitHub Actions. No local build/test/lint/generator.

## 5. Current state
source/shared/node/mcp/mcp-marketplace.ts reads Dashboard catalog; desktop-mcp-manager.ts owns the production manager and routed tool facade. frontend PluginsDesktopSurface loads catalog, effective plugins and server state together. The existing encrypted SandUserSecretsStore supports dedicated store paths and account scopes. On 2026-10-09 the user authorized completing and publishing the usable marketplace and retrieving/provisioning needed configuration through the unified-device-control Mac. The existing identity OAuth config requests only login scopes and discards service tokens; account_connections is the existing service-connection schema. Mac Google Cloud has Chrome publishing clients, whose suitability is unconfirmed; personal and organization GitHub OAuth App lists are empty. Google preview entitlement has not been confirmed.

## 6. Target state
Existing Plugins entry displays the Fabushi official catalog. Install configures a real remote connector; actual authorization and MCP discovery determine status/tool count. The directory remains available during foreign Dashboard failures. Provider connection OAuth, refresh and disconnect are required through the existing platform Worker and encrypted native MCP edge, with separate grants from Fabushi sign-in. Registered-client configuration and real provider evidence must precede a fully usable claim.

## 7. Architecture / ownership
Catalog metadata lives in shared MCP. Native connection state, encrypted vault and transport are narrow Electron MCP edges, composed by desktop-mcp-manager.ts. The existing independent Coordinator -> Host routed tools path remains the caller; no second Agent/runtime or Mini App market is introduced. Legacy plugins retain their current owner.

## 8. Contracts / data flow
Catalog IDs use fabushi-official-*; server IDs use the same namespace. Native installed records contain plugin ID, token, account label and disabled tools in encrypted JSON. Renderer receives only public metadata/status/tools. Unknown IDs cannot select arbitrary remote hosts. Install values accept optional ACCESS_TOKEN; updates with absent/blank token preserve the current token. Disconnect deletes credentials; uninstall deletes the install. Connection OAuth broker routes belong to the existing platform Worker: authenticated start/poll, state-bound public callback, authenticated refresh/revoke. Provider tokens remain isolated from Fabushi sessions; client secrets stay server-side. Missing registration or preview entitlement returns an actionable provider configuration error. Successful tools/call is projected into canonical generated MCP results. Tool discovery/calls are fenced against current account and install revision.

## 9. Constraints
HTTPS fixed endpoints, no redirects, 60s request timeout, bounded response size, provider credentials only. No automatic retries of writes. Offline public catalog; no pretend connection or hardcoded tool inventory.

## 10. Failures / edge cases
401/403 -> needsAuth; preview denial is a provider error. Missing secure storage -> session-only state via the existing vault, never plaintext persistence. Logout/account change/removal invalidates pending output. Legacy failures produce a visible warning. Pagination has a bounded page count and rejects repeated cursors. Unsupported content types/protocol errors fail explicitly.

## 11. Implementation strategy
Commit this spec first. Add provider-source catalog; add injectable native connection/transport owner with existing encrypted vault adapter; compose catalog/install/server/tool lifecycle in the current production facade. Repair renderer partial-load handling. Add Actions contracts and existing renderer/Electron build checks. Publish a reviewable PR, inspect exact-head results, then integrate only when required gates pass.

## 12. Verification
Actions unit/contract tests: all official endpoints/provenance; install without credentials; update/disconnect/uninstall; account isolation and stale-result fencing; no account token sent to provider; JSON/SSE MCP initialization and discovery/call; redirects/errors/no write retry; partial-load UI state. Existing production renderer and Electron compile/build gates cover shipping composition.

## 13. Acceptance
- AC-1: catalog visible in existing Plugins, with all nine entries and honest provider/preview labeling.
- AC-2: account-scoped encrypted install/update/remove and real MCP tool discovery/call pass exact-head Actions.
- AC-3: Coordinator/Host uses enabled installed connector tools; disabled tools cannot execute.
- AC-4: fresh packaged product connects GitHub and Google through registered Fabushi OAuth, handles disconnect/expiry/re-auth, and records exact-head live evidence.
- AC-5: current-head CI, integration and release verified. Code push alone is not completion.

## 14. Release / rollback
Deliver through a dedicated main-based PR. Reuse account_connections for service identities; any additional OAuth attempt/credential lifecycle fields require versioned ACCOUNT_DB migrations. Client secrets and provider tokens must never enter git or public release artifacts. Existing installs are retained. Rollback removes official connector composition without altering legacy installs; users may remove the dedicated encrypted native installs.

## 15. Observability / evidence
Retain exact commit/run/job/step links. Emit sanitized error classes and partial-load warnings; never log tokens, raw auth responses or secrets. Live OAuth/configuration blocks remain explicit.

## 16. References
User request: create Fabushi official marketplace and list common Google/GitHub MCPs.
Canonical Plugins requirements CONN-001..008,010 in docs/specs/grok-bot-018-runtime-product-parity-recovery.md.
https://developers.google.com/workspace/guides/configure-mcp-servers
https://developers.google.com/workspace/gmail/api/guides/configure-mcp-server
https://github.com/github/github-mcp-server
https://github.com/github/github-mcp-server/blob/main/docs/host-integration.md

## 17. Spec compliance
| Requirement / AC | Status | Evidence / reason |
| --- | --- | --- |
| AC-1..3 | implemented; live acceptance pending | Head 1e800e28dac6c194213bda0cec57adecbd9c1a57: provider protocol/lifecycle fixtures and production compile/build gates pass; no live provider authorization claimed |
| AC-4 | blocked | User confirmed OAuth applications are not configured; Google preview entitlement and packaged-product OAuth evidence remain outstanding |
| AC-5 | blocked | Reviewable draft PR #49; integration/release intentionally pending OAuth/live acceptance |

## 18. Implementation evidence
- Draft delivery: https://github.com/bhrumom/fabushi-desktop/pull/49
- Provider contracts: https://github.com/bhrumom/fabushi-desktop/actions/runs/37759000207 (10 protocol/lifecycle fixture tests; no real provider account calls).
- Production Electron/renderer, architecture and runtime checks: https://github.com/bhrumom/fabushi-desktop/actions/runs/37759000216; renderer job 113250536920 passes all steps, including strict architecture inventory and production bundling.
- Canonical source changes remain confined to existing Plugins and Coordinator/Host facade. The semantic-adaptations registry records the five changed frozen boundaries with behavior and test evidence.

## 19. Remaining setup and live acceptance
1. Register Fabushi's GitHub OAuth App/GitHub App and Google Cloud OAuth client under the product owner's accounts, with selected redirect URLs. Do not put client secrets or user access tokens into source or email.
2. Obtain/confirm Google Workspace MCP Developer Preview access and enable each required API and MCP API in the qualified project. Google Chat additionally requires its Chat app configuration.
3. Implement registered-client OAuth callback, state/PKCE as appropriate, encrypted token refresh/revocation and expiry/re-auth in the existing native auth edge; the interim ACCESS_TOKEN field is not a replacement for this acceptance item.
4. Run fresh packaged-product sign-in, discovery, read/write permission, disable, disconnect, expiry and account-switch acceptance against real GitHub and Google accounts in the approved cloud environment. Retain exact-head Actions evidence before integration/release.

## 20. Provider OAuth production design (2026-10-09)
- Existing platform Worker owns /api/mcp/oauth/start, /authorize, /callback, /attempts/:attempt_id and /cancel, plus /api/mcp/connections/:connection_id/refresh and /revoke. Start/poll/cancel/refresh/revoke require a live session-backed Fabushi account. Public browser tickets and callback states are hashed, expiring, single-use and bound to account, session and connector. Google and GitHub registration use https://api.ombhrum.com/api/mcp/oauth/callback, independently of login callbacks.
- Reuse the existing account_connections principal owner. A versioned additive migration holds short-lived attempts and native credential digests; credentials never become relational connection metadata. Token delivery payloads use authenticated AES-256-GCM encryption with a domain-separated key derived from the existing server signing private key, a fresh nonce and attempt-bound AAD; expiry, consumption and cancellation erase ciphertext. Signing-key rotation may cancel pending deliveries; it does not invalidate native credentials.
- Native vault stores returned access/refresh tokens, expiry and connection id. Native background polling never returns tokens to renderer. Proactive refresh occurs before tool discovery/execution; refresh is single-flight per connector and fenced by account/install revision. Refresh errors require explicit reauthorization; uncertain MCP writes are not retried. Broker credential digests bind refresh/revoke to the actual connected grant. Disconnect removes native credentials immediately and reports any unresolved provider revocation. Uninstall cancels attempts and revokes its grant.
- Google asks only the selected connector scopes plus OpenID identity, offline access and explicit provider consent. GitHub connection requests repository access separately from sign-in. Dedicated MCP clients may be used without changing existing identity-login clients. OAuth application consent, Google preview membership/API enablement, Chat configuration and any provider verification remain real external prerequisites.
- Actions must compile the actual wasm Worker, exercise SQLite migration/attempt/ownership constraints and native OAuth completion/refresh/cancellation/account-switch contracts, then run existing production gates. Deploy only the verified exact source; provider and packaged acceptance must pass before reporting fully usable.

## 21. Canonical repository ownership
Platform Worker OAuth implementation and ACCOUNT_DB migration belong in this canonical Core repository. Native vault/polling/transport and the existing Plugins UI belong in Desktop PR #49. Public api.ombhrum.com gateway and all production deployment orchestration belong in fabushi-backend. The legacy fabushi repository must remain read-only. No product Worker changes are delivered from the Desktop snapshot. Broker verification runs in this repository's GitHub Actions against the exact source; Backend deploys the verified canonical Core main SHA explicitly.
