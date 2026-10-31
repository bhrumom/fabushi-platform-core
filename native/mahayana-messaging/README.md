# Mahayana Messaging Runtime

Fabushi self-hosted messaging foundation.

Goals:

- one Rust message runtime for humans, AI agents and bots;
- no Telegram server dependency;
- contacts, bots, groups, channels and automated agents share one actor model;
- desktop Electron, mobile native clients and web clients consume the same events.

The implementation is inspired by mature messaging UX patterns, but protocol, storage, identity and payment contracts are Fabushi-owned.

## Domains

- actor: user, bot, assistant, service identity;
- conversation: direct chat, group, channel, agent thread;
- message: text, media, tool events, payments and app events;
- mini app: sandboxed application sessions;
- commerce: invoices, settlement and entitlement events.

## Migration rule

Do not import GPL client implementations into this crate. Reimplement required capabilities behind Fabushi contracts.
