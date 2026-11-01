# Telegram WebProxy relay stream boundary — Specification

Status: active
Owner: canonical `telegram-network` transport owner; `telegram-protocol` remains unchanged
Last updated: 2026-11-01
Related issue/task/PR: TDRP-001 / task 7457eb1c-11a3-4626-8563-a3e495872536 round 79

## 1. Context / problem

Stage A is migrating fixed `telegramdesktop/tdesktop@f985723b552b30fe1c17d377d57f44e61a15a8c4` responsibilities without running CI or executable validation. The preceding TLS-camouflage closure deliberately left adjacent WebProxy transport work open.

The fixed-U WebProxy path is:

`TcpConnection` -> `WebProxySocket` logical byte stream -> process-wide `WebProxy::Transport` -> WebView/browser carrier -> relay.

This review asks whether stream framing, flow-control window encoding/state, or the WebView nonce boundary contains an exact responsibility that can be carried by an existing Fabushi owner without importing the broader WebProxy scheduler/carrier lifecycle.

## 2. Goal

R1. Identify the exact fixed-U relay frame grammar and stream-window boundary.

R2. Decide existing ownership without introducing a second Telegram runtime, provider, transport, scheduler, or request-history owner.

R3. Separate independently specifiable wire rules from WebProxy carrier/WebView lifecycle.

R4. Record whether a production delta is valid on the current shipping Fabushi call graph.

R5. Preserve Stage-A `validation_status=not-run` and `accepted=false`.

## 3. Non-goals / out of scope

- TLS camouflage ClientHello/ServerHello/application-record wire already closed in the preceding responsibility.
- Quick-ACK, protocol DC-id, 64-byte connection-start nonce/secret/AES-CTR, MTProto framing/envelope/handshake.
- auth-key binding/persistence.
- connection racing, reconnect, retry, carrier selection, WebView retry/adoption scheduling, browser fallback scheduling.
- arbitrary request history, gzip, complete difference/differenceSlice decoding/application.
- WebProxy settings/UI/link parsing as a product surface.
- Stage-B build/test/runtime/package/deployment validation.

## 4. Requirements

### Relay frame wire

R6. A relay frame header is exactly 8 bytes:
- byte 0: frame type;
- bytes 1..3: unsigned 24-bit stream id, big-endian;
- bytes 4..7: unsigned 32-bit payload length, big-endian;
- remaining bytes: payload.

R7. Known fixed-U frame types are:
- `Open=0x01`
- `Data=0x02`
- `Close=0x03`
- `Window=0x04`
- `Ping=0x05`
- `Pong=0x06`
- `Hello=0x10`
- `Welcome=0x11`
- `AuthChallenge=0x12`
- `AuthResponse=0x13`
- `Bye=0x1f`.

R8. Encoding rejects a stream id above `0x00ff_ffff` and a payload above 1 MiB. Incremental parsing rejects unknown types and payload lengths above 1 MiB, preserves an incomplete trailing frame in the input buffer, and rejects a completed batch above 4096 frames.

### Stream/window semantics

R9. `Window` payload is exactly one nonzero unsigned 32-bit big-endian amount. Each stream begins with a 4 MiB send window and a 4 MiB receive window.

R10. Received nonempty `Data` must not exceed the current receive window and consumes that receive credit. Credit is granted back only for bytes actually drained from `WebProxySocket::read`. A grant that would raise receive credit above the 4 MiB initial ceiling is a protocol error. Incoming `Window` credit increases send credit, saturating at `u32::MAX`.

R11. The fixed-U transport coalesces outbound receive-credit grants at 256 KiB or 20 ms and caps individual outbound `Data` frames at 64 KiB. Those timers/queues are transport lifecycle policy, not part of the reusable scalar frame codec.

### WebView nonce boundary

R12. Hidden-WebView bootstrap creates 32 secure-random bytes, encodes them as URL-safe base64 without padding, places them in `#android=<nonce>`, and accepts the initial `tproxy-android-init` control only when `v == 1` and the supplied nonce exactly matches. Only then does the carrier enqueue `Hello(stream=0,payload=[1])`.

R13. The nonce check is not an independent network-wire codec. It is bound to WebView navigation/origin restrictions, bridge initialization, adoption timeout/health, write acknowledgement, failure/retry, and carrier lifecycle.

### Ownership / reachability

R14. Relay multiplexing is a carrier/network responsibility. It must belong to the existing `telegram-network` owner if/when a shipping WebProxy carrier is introduced. It must not be placed in `telegram-protocol`, whose current responsibility is reusable MTProto wire/cryptography above the carrier.

R15. On the live platform-core checkpoint, there is no shipping WebProxy logical-stream/carrier implementation or selectable WebProxy call path. Adding only a relay codec or window helper would therefore create dead, non-shipping production API. This round MUST NOT manufacture that code.

R16. Production implementation remains blocked until an existing `telegram-network` shipping entrypoint can instantiate/select a WebProxy carrier and consume the exact frame/window contract without importing broader scheduler ownership into `telegram-protocol`.

## 5. Current state

Live canonical checkpoint before this spec:
`bhrumom/fabushi-platform-core checkpoint/tdrp-telegram-auth-lifecycle-7457eb1c-r16@c0f2a6c4098cc5b54f3ee580db078e55c61982ab`.

Canonical `native/telegram-network/src/lib.rs@241cd8ec792070a8de8af9ec6ca5b317e34376d1` owns the current socket/TcpTransport path but has no WebProxy relay stream/carrier module. Its authored external contract remains `native/telegram-network/tests/tcp_transport.rs@71f9732b1c81d7bf782ff1eccb2852b1401cfa6f`.

The desktop provider mirrors those two blobs byte-for-byte. There is no current WebProxy production caller to which an isolated frame codec could be wired.

## 6. Target state

For this round, the target state is deliberately a mapped, blocked responsibility with no production behavior delta:
- exact fixed-U frame/window rules are durably specified;
- `telegram-network` is the selected future existing owner;
- `telegram-protocol` remains unchanged;
- WebView nonce/bootstrap remains with the future WebProxy carrier lifecycle;
- no dead codec/test facade is created merely to claim implementation.

When a shipping WebProxy carrier is introduced, this spec is the minimum wire/flow-control contract that implementation must consume.

## 7. Architecture and ownership boundaries

`telegram-protocol`:
- MTProto TL/message/crypto/transport framing already owned there;
- no WebProxy relay frame or WebView nonce ownership added.

`telegram-network`:
- correct existing owner for WebProxy relay multiplexing and logical-stream flow control;
- future carrier I/O, stream registration, relay frame encode/decode, send/receive windows, and lifecycle integration belong here.

Broader scheduler/carrier lifecycle:
- carrier selection, WebView/browser construction, retry/backoff, reconnect, browser fallback, health timers, write-ack timeout and global activation remain outside this isolated exact-wire review.

## 8. Interfaces / contracts / schemas / data flow

Authored source-level contract vectors, not executed:

1. `Open(stream=1,payload=[])` -> `01 00 00 01 00 00 00 00`.
2. `Data(stream=0x00ff01,payload=aa bb)` -> `02 00 ff 01 00 00 00 02 aa bb`.
3. `Window(stream=0x123456,amount=0x01020304)` -> `04 12 34 56 00 00 00 04 01 02 03 04`.
4. An incomplete header or declared payload is retained for the next input chunk rather than emitted.
5. Unknown type, payload > 1 MiB, or more than 4096 completed frames fails closed.
6. `Window` with payload length other than 4 or amount zero is invalid at the stream semantic boundary.
7. Before `Welcome(stream=0,payload=[])`, no other relay frame is accepted.
8. `Data` after a stream is remembered closed may be discarded as valid late traffic; unknown never-opened stream traffic is a protocol error.
9. WebView `tproxy-android-init` must match version 1 and the exact 32-byte-derived URL nonce before `Hello(0,[1])` is queued.

No executable test file is added in this blocked round because there is no production WebProxy symbol to test. Creating a test-only codec would violate R15.

## 9. Constraints and non-functional requirements

- Stage A: no GitHub Actions/status/check query or trigger; no build, typecheck, lint, validator, generator, unit/contract/integration/E2E execution, package, deployment, installation, benchmark, fuzz or runtime acceptance.
- No force push.
- No bespoke cryptography.
- No second Telegram product/runtime/provider.
- No widening into broader scheduler/reconnect/request-history ownership.

## 10. Failure modes and edge cases

- invalid/unknown frame type;
- stream id above 24-bit range on encode;
- payload length above 1 MiB;
- incomplete frame split across input chunks;
- batch above 4096 frames;
- empty `Data`;
- zero/malformed `Window`;
- receive-window overrun or overgrant;
- window arithmetic overflow;
- traffic before `Welcome`;
- traffic for unknown stream ids;
- duplicate/invalid `Welcome`;
- WebView init version/nonce mismatch;
- dead production codec with no shipping caller.

## 11. Implementation strategy

This round is a no-production-delta / blocked disposition.

1. Keep current network/protocol production blobs unchanged.
2. Record this spec first.
3. Record a unique desktop parity-ledger row as `mapped / pending / validation_status=not-run / accepted=false`.
4. Record the blocking shipping condition and reviewed nonce boundary in P0/source-closure evidence.
5. Do not create a test-only or unused production module.

A later round may implement this only after it establishes a real existing-owner shipping WebProxy carrier call chain.

## 12. Verification / test strategy

Stage A: source review only. The vectors in §8 are authored contracts but are not executed.

Future Stage B, after production reachability exists:
- unit/table tests for every frame type, length boundary, chunk split and 4096-frame cap;
- property tests for incremental parse/serialize round-trip;
- state tests for Welcome/Open/Data/Window/Close and late closed-stream traffic;
- flow-control tests for exact 4 MiB credit conservation and u32 saturation;
- nonce/bootstrap negative tests;
- integration tests through the real WebProxy carrier;
- fault tests for malformed frames, carrier loss and bounded queues.

## 13. Acceptance criteria / Definition of Done

AC-1. Fixed-U frame/window/nonce behavior is anchored to exact source blobs.
AC-2. Existing-owner resolution is explicit and no duplicate owner is introduced.
AC-3. No dead production delta is manufactured while shipping reachability is absent.
AC-4. The desktop unique ledger/P0/source-closure records this responsibility and blocker.
AC-5. Stage-A executable validation remains not-run and acceptance remains false.
AC-6. Production implementation remains blocked until a real `telegram-network` WebProxy carrier entrypoint exists.

## 14. Release / migration / rollback

No runtime release/migration occurs in this round. Rollback is deletion/reversion of this spec and corresponding desktop bookkeeping only; production blobs are unchanged.

## 15. Observability / evidence

Required evidence is Git object identity:
- platform spec commit/blob;
- unchanged canonical network production/test blobs;
- desktop mirror blob identity;
- unique parity-ledger row and final desktop checkpoint;
- fixed-U source blob identities.

No runtime telemetry/evidence is claimed.

## 16. References / provenance

Fixed U: `telegramdesktop/tdesktop@f985723b552b30fe1c17d377d57f44e61a15a8c4`.

- `Telegram/SourceFiles/mtproto/web_proxy/web_proxy_frame.h@a5ba491fed67d70aee88e6ec2e3ac47c991e78f3`
- `Telegram/SourceFiles/mtproto/web_proxy/web_proxy_frame.cpp@bd1d7b310f40203f9da914efc8bdd6711035884b`
- `Telegram/SourceFiles/mtproto/web_proxy/web_proxy_transport.h@f58850d2bc2ceabd93113d7725903e1c0d92d71b`
- `Telegram/SourceFiles/mtproto/web_proxy/web_proxy_transport.cpp@e25ad970e84686e9aca061d02c274048793b5ad4`
- `Telegram/SourceFiles/mtproto/details/mtproto_web_proxy_socket.cpp@9aa05e7034031310de4aa9d410fbb15d21c63ee8`
- `Telegram/SourceFiles/mtproto/details/mtproto_web_proxy_socket.h@02ed27b77d4ff606b3e1cb548a60e4f2c2ca11f0`
- `Telegram/SourceFiles/mtproto/connection_tcp.cpp@d721a9058e1885d2964ded55bd4e4c7c2ce9e381`
- `Telegram/SourceFiles/mtproto/web_proxy/web_proxy_webview.cpp@8a2efc8fb15e400366f199cd956effc8ad362da8`
- `Telegram/SourceFiles/mtproto/mtproto_proxy_data.cpp@f9748fcb6358dd8ce7429536ec2b999cb2a989ef`

No upstream C++ source is copied into Fabushi by this review.

## 17. Spec compliance record

| Requirement / AC | Status | Evidence / reason |
| --- | --- | --- |
| R1-R5 | passed | Exact fixed-U/live-owner discovery recorded above. |
| R6-R8 | passed | Exact relay frame grammar captured from `web_proxy_frame.{h,cpp}`. |
| R9-R11 | passed | Window/data credit behavior captured from `web_proxy_transport.cpp` and `WebProxySocket::read`. |
| R12-R13 | passed | WebView nonce/init boundary captured from `web_proxy_webview.cpp`; classified lifecycle-bound. |
| R14 | passed | Existing-owner-first resolves relay multiplexing to `telegram-network`, not `telegram-protocol`. |
| R15 | passed | Live network owner and desktop mirror have no WebProxy carrier/caller; no dead production code added. |
| R16 | blocked | Requires a real shipping WebProxy carrier entrypoint before production implementation is valid. |
| AC-1 | passed | Exact fixed-U blob identities are listed in §16. |
| AC-2 | passed | No duplicate owner or runtime introduced. |
| AC-3 | passed | Production blobs intentionally unchanged. |
| AC-4 | blocked | Completed only after desktop ledger/P0/source-closure synchronization. |
| AC-5 | passed | No executable validation is run in this round. |
| AC-6 | blocked | Shipping WebProxy carrier is not present on the current Fabushi call graph. |
