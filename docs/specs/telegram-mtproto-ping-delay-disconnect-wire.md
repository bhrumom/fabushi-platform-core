# Telegram MTProto ping_delay_disconnect exact wire request

Status: Stage A implementation contract; executable validation deferred.

## Context and scope

Fixed upstream is `telegramdesktop/tdesktop@f985723b552b30fe1c17d377d57f44e61a15a8c4`.

The fixed-U MTProto schema declares:

- `ping#7abe77ec ping_id:long = Pong`
- `ping_delay_disconnect#f3427b8c ping_id:long disconnect_delay:int = Pong`

In fixed-U `Telegram/SourceFiles/mtproto/session_private.cpp@13cbc76ec723386027b5b4dcac79999b791c4614`, the main connected session serializes `ping_delay_disconnect` with the current ping id and `kPingDelayDisconnect = 60`; it also schedules the next/forced ping through session timers. The current Fabushi shared protocol owner already has strict `pong#347773c5` decoding and the network owner has a single-outstanding-ping path, but the protocol owner has no exact builder for `ping_delay_disconnect`.

The immediately preceding fixed-U `tlsClientHello/tlsBlock*` declarations were also reviewed. They are not inbound MTProto wire objects in Telegram Desktop: `mtproto_tls_socket.cpp@e7562e3ca1a1da03f5d368085d5e61d887ab3f16` constructs them locally as a typed ClientHello template and `Generator::Part::writeBlock` consumes that AST to generate TLS camouflage bytes. Fabushi's current canonical network owner exposes direct `TcpTransport` only and has no equivalent TLS camouflage socket/generator lifecycle. Therefore this spec does not invent a TL parser for those local template objects; the TLS camouflage lifecycle remains fail-closed/open.

## Goal

Add a source-neutral exact serializer for the fixed-U `ping_delay_disconnect` request to the existing shared `telegram-protocol` owner, preserving the raw `ping_id:i64` and `disconnect_delay:i32` values in schema order.

## Non-goals

- Do not synthesize Telegram Desktop's connected/main-session selection policy.
- Do not add ping timers, forced-ping scheduling, reconnect behavior, or connection-age state.
- Do not add multi-ping history or arbitrary historical sent-request lookup.
- Do not add `wasSent/haveSent/_resendingIds/toSend/_sentContainers` semantics.
- Do not implement TLS camouflage, TLS ClientHello generation, proxy/bootstrap discovery, gzip handling, or complete updates difference decoding.
- Do not create a second Telegram runtime, transport, provider, or session owner.

## Requirements

- R1: The canonical protocol owner SHALL export `PING_DELAY_DISCONNECT_CONSTRUCTOR = 0xf3427b8c`.
- R2: The canonical protocol owner SHALL expose `build_ping_delay_disconnect(ping_id: i64, disconnect_delay: i32) -> Vec<u8>`.
- R3: The builder SHALL encode exactly constructor, `ping_id`, then `disconnect_delay`, all in TL little-endian fixed-width order.
- R4: The builder SHALL preserve all raw signed values without imposing timer or policy validation.
- R5: Authored-only contracts SHALL cover normal fixed-U values, signed boundary values, exact length, constructor bytes, and field order.
- R6: The protocol slice SHALL have no socket/session/timer side effects.
- R7: Desktop provider protocol sources SHALL be synchronized byte-for-byte from the canonical implementation.
- R8: The Telegram Desktop main-session/timer lifecycle for deciding when to use `ping_delay_disconnect` SHALL remain explicitly open/fail-closed until the existing network owner can represent it precisely.
- R9: The fixed-U TLS ClientHello template/generator lifecycle SHALL remain explicitly open/fail-closed; no artificial inbound parser SHALL be introduced for the local `tlsClientHello/tlsBlock*` AST.
- R10: Stage A SHALL author contracts only and SHALL NOT run or query build, typecheck, lint, schema validator, generator, unit/contract/integration/E2E tests, package, deploy, installation, GitHub Actions, checks, or statuses.

## Architecture and ownership

- Exact MTProto request serialization: existing `native/telegram-protocol`.
- Existing single-outstanding-ping correlation: `native/telegram-network::EstablishedSession`; unchanged by this wire-only slice.
- Desktop consumer copy: existing `source/mahayana/mahayana-rs/providers/telegram-protocol`.
- Ping scheduling/main-session decision: remains with the network/session lifecycle and is not synthesized here.
- TLS camouflage ClientHello generator/socket lifecycle: no precise existing owner today; remains blocked/fail-closed.

## Implementation strategy

1. Add the fixed-U constructor constant and exact builder to `native/telegram-protocol/src/api.rs`.
2. Re-export both from `native/telegram-protocol/src/lib.rs`.
3. Author byte-level builder contracts in the existing protocol test module without executing them.
4. Durably commit the canonical implementation.
5. Synchronize the canonical protocol files byte-for-byte into the existing desktop provider.
6. Update the existing TDRP ledger/P0/source-closure/evidence with `implemented / validation_status=not-run / accepted=false`, while recording the TLS camouflage and ping scheduling lifecycle blockers.

## Verification strategy

Stage A source review only:

- compare constructor and field order to fixed-U `mtproto.tl`;
- inspect authored byte-level contracts;
- confirm no timer/socket/session mutation was added;
- confirm desktop provider api/lib blobs equal canonical blobs;
- record all executable evidence as not run.

## Acceptance criteria

- AC1: exact constructor and builder exist in canonical protocol owner.
- AC2: source inspection shows constructor + i64 + i32 encoding with no extra bytes or policy transformation.
- AC3: boundary-value and exact-layout contracts are authored but not run.
- AC4: desktop provider api/lib blobs are byte-identical to canonical blobs after synchronization.
- AC5: ledger/P0/source-closure/evidence record the responsibility as implemented, validation not run, accepted false.
- AC6: TLS camouflage ClientHello template/generator lifecycle and main-session/timer decision remain explicit fail-closed blockers.
- AC7: Stage-B executable validation remains pending.

## References

- `Telegram/SourceFiles/mtproto/scheme/mtproto.tl@76225965bfc794388db241f3f760db8ab79e84f8`
- `Telegram/SourceFiles/mtproto/session_private.cpp@13cbc76ec723386027b5b4dcac79999b791c4614`
- `Telegram/SourceFiles/mtproto/details/mtproto_tls_socket.cpp@e7562e3ca1a1da03f5d368085d5e61d887ab3f16`
- `docs/specs/telegram-mtproto-single-ping-pong.md`
- `docs/specs/spec-first-ai-development.md`


## Source-level implementation review

Canonical implementation commit: `cb816c7540ea93717238b3aec96372cdab1c1706`. Canonical blobs: protocol api `405b04166bea039051833a38ad0a75c62107f70c`, protocol lib `ef7fe73335551e303fa71b02f5cf066d778ae8a4`. Desktop synchronization commit: `d38f6bef11a9a4334f8387c73c7b74e7994c0cdb`; desktop bookkeeping commit: `df7cd04aa812c9c8cd5f5c9916f0d9261ec37b8f`.

- R1-R4: satisfied by source inspection — `PING_DELAY_DISCONNECT_CONSTRUCTOR` is exactly `0xf3427b8c`; `build_ping_delay_disconnect` writes constructor, `ping_id:i64`, then `disconnect_delay:i32` through the existing TL writer and applies no policy transformation.
- R5: satisfied by source inspection — authored-only contracts check exact 16-byte field layout, fixed-U delay 60, and signed minimum/maximum values.
- R6: satisfied by source inspection — the builder touches no socket, timer, session, request history, persistence, or runtime state.
- R7: satisfied by remote blob readback — desktop provider api/lib blobs are byte-identical to the canonical blobs above.
- R8: satisfied as an explicit boundary — no main-session timer/scheduling behavior was synthesized; that lifecycle remains open/fail-closed.
- R9: satisfied as an explicit boundary — fixed-U `tlsClientHello/tlsBlock*` were verified as the local ClientHello-template AST consumed by `mtproto_tls_socket.cpp`; no artificial inbound parser or second TLS transport owner was introduced. The missing TLS camouflage socket/generator lifecycle remains open/fail-closed.
- R10: satisfied for Stage A process — no build, typecheck, lint, schema validator, generator, unit/contract/integration/E2E, package, deployment, installation, GitHub Actions, checks, or statuses were run or queried.

Acceptance criteria AC1-AC6 are satisfied at source/implementation/bookkeeping level. AC7 remains intentionally **blocked/not-run** until Stage B executable validation.
