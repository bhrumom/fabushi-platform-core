# Mahayana Runtime

Mahayana is Fabushi's product-owned, provider-neutral agent runtime. Its public
contracts, lifecycle, capability model, policies, conversation routing,
Telegram, MiniApps, FFI/WASM, CLI and host integrations are owned here and do
not belong to any external agent vendor.

The repository currently retains upstream Codex sources in `../codex-rs` as a
compatibility implementation while the native Mahayana kernel is completed.
That compatibility layer is intentionally replaceable: new product surfaces
must depend on Mahayana contracts rather than `codex-*` protocol types. See
`../../../docs/mahayana-sovereign-kernel.md` for the migration architecture.

The runtime contract is conversation-first. Every surface sends the same
commands and receives the same events whether the selected peer is an AI
agent, a Telegram contact, a Mahayana friend, a bot, or a MiniApp.

`mahayana-kernel` is the provider-neutral execution boundary. It owns session
and operation IDs, capabilities, execution policy, events, backend discovery
and routing. Codex and future Grok-derived compatibility implementations attach
behind that boundary; native Mahayana engines are preferred as they reach
feature parity.

Build profiles are intentionally explicit:

- `desktop-full`: native filesystem/process/Git plus all granted providers.
- `mobile-embedded`: in-process agent with app-sandbox tools and no arbitrary
  process execution.
- `web-wasm`: browser-local runtime, storage, and Web Worker transport.

No profile is allowed to silently switch to a remote Agent gateway. A remote
model endpoint is a model provider, not an Agent runtime, and must be visible
in runtime status.

Third-party implementations keep their original license and provenance. A
Mahayana-owned architecture does not erase attribution for reused Apache-2.0
code; independence is achieved by owning the stable contracts and replacing
upstream implementations capability-by-capability.
