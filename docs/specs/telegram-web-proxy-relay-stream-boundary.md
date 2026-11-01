# Telegram WebProxy relay stream boundary — Specification

Status: active; Stage-A implementation authorized
Owner: canonical `telegram-network` transport owner; `telegram-runtime` remains the existing session owner; product Settings remains the existing durable settings owner
Last updated: 2026-11-01
Related issue/task: TDRP-R9-MTPROTO-WEB-PROXY-RELAY-STREAM-001 / task 7457eb1c-11a3-4626-8563-a3e495872536 round 101

## 1. Context

Stage A fixes upstream at `telegramdesktop/tdesktop@f985723b552b30fe1c17d377d57f44e61a15a8c4` and forbids executable validation. The earlier round-79/110 disposition correctly blocked production relay code because no selected Web carrier could be reached from the shipping runtime. That premise is no longer true.

The live shipping chain now carries a source-neutral selected proxy projection through the existing owners:

`ProductSettingsOwner v2 -> TelegramClientLease -> RuntimeClient.network_proxy -> telegram-network`.

`NetworkProxyKind::Web` is therefore selectable on the same production call graph that already selects MTProto proxy transport. The old “no selectable Web caller” blocker is superseded. This specification authorizes the relay lifecycle inside the existing `telegram-network` owner without creating a second runtime, provider, settings store, or session owner.

## 2. Goals

R1. Implement the exact fixed-U relay frame grammar and stream/window state machine in `telegram-network`.

R2. Compose a selected Web proxy as a logical byte-stream carrier beneath the existing MTProto obfuscation/framing code rather than introducing another Telegram runtime.

R3. Preserve product Settings as the only durable proxy owner and `RuntimeClient/SessionOwner` as the only Telegram runtime/session owner.

R4. Provide a bounded browser bootstrap carrier using the fixed-U browser/WebView bridge semantics. Platform launch is a thin desktop adapter; relay state remains in `telegram-network`.

R5. Write contracts in Stage A but do not run them. Implementation status may be `implemented / validation_status=not-run / accepted=false`, never verified.

## 3. Fixed-U relay wire

R6. A relay frame header is exactly 8 bytes:
- byte 0 frame type;
- bytes 1..3 unsigned 24-bit stream id, big-endian;
- bytes 4..7 unsigned 32-bit payload length, big-endian;
- payload bytes follow.

R7. Known types are:
`Open=0x01`, `Data=0x02`, `Close=0x03`, `Window=0x04`, `Ping=0x05`, `Pong=0x06`, `Hello=0x10`, `Welcome=0x11`, `AuthChallenge=0x12`, `AuthResponse=0x13`, `Bye=0x1f`.

R8. Stream ids are limited to `0x00ff_ffff`; payloads are limited to 1 MiB; completed batches are bounded to 4096 frames; incomplete input remains buffered; unknown types fail closed.

## 4. Stream/window lifecycle

R9. Each logical stream starts with a 4 MiB send window and 4 MiB receive window.

R10. Before `Welcome(stream=0,payload=[])`, no relay frame other than that Welcome is accepted. Welcome opens all registered streams by sending `Open`.

R11. Incoming nonempty `Data` must fit the remaining receive window and consumes that credit. Credit is returned only after the local MTProto consumer actually reads bytes from the logical stream.

R12. `Window` payload is exactly one nonzero unsigned 32-bit big-endian amount. Incoming credit increases the send window with saturation at `u32::MAX`. A local receive-credit grant that would exceed the initial 4 MiB ceiling is a protocol error.

R13. Outbound Data is capped at 64 KiB per frame. Receive-credit grants coalesce at 256 KiB or a 20 ms flush. Closed stream ids are remembered with a bounded 4096-entry tombstone window so late Data/Window/Close may be ignored while traffic for a never-known stream fails closed.

R14. `Ping(stream=0)` payload is limited to 64 bytes and is answered with `Pong`. `Bye(stream=0)`, malformed relay input, duplicate/invalid Welcome, receive-window overrun, malformed Window, or unknown stream traffic tears down the active carrier and all logical streams.

## 5. Bootstrap/carrier composition

R15. Web proxy configuration is valid only when:
- the selected kind is Web;
- port is 443;
- host/base path/secret satisfy the existing canonical WebProxy capability contract in `telegram-protocol::web_proxy`;
- the bridge URL is produced by the existing domain-separated HMAC capability helper.

R16. The native relay owner exposes a loopback browser bootstrap with a random one-shot capability and same-origin local transport. The browser page embeds the canonical remote bridge URL, transfers a `MessagePort` using `{t:"tproxy-init",v:1}`, queues binary relay traffic until the remote bridge is initialized, and then forwards relay payloads bidirectionally.

R17. Browser/local-carrier authentication is bounded and secret-bearing tokens are never returned in status/debug surfaces. A carrier replacement or loss clears Welcome state, queues, closed-stream tombstones and active logical streams before a new carrier can be adopted.

R18. The fixed-U hidden-WebView alternative uses 32 secure-random bytes encoded base64url without padding in `#android=<nonce>`, accepts only `tproxy-android-init` with `v==1` and exact nonce, and only then sends `Hello(0,[1])`. The shipping desktop implementation may use the fixed-U external-browser fallback rather than duplicate WebView ownership, but its Hello/Welcome and relay state machine MUST be identical.

R19. The desktop-specific action of opening the loopback bootstrap URL belongs only to the existing desktop host/platform adapter. It MUST NOT become a Telegram settings owner or persist carrier state.

## 6. Ownership and call graph

`telegram-network` owns:
- relay frame encode/decode;
- logical stream ids and lifecycle;
- send/receive windows;
- closed-stream tombstones;
- bounded carrier queues;
- loopback browser bridge;
- carrier generation/adoption/loss;
- mapping selected Web projection to a logical byte stream.

`telegram-protocol` continues to own:
- WebProxy canonical host/base-path/secret validation;
- bridge capability/path/URL derivation;
- MTProto obfuscation, framing, crypto, DC-id wire rules.

`telegram-runtime` continues to own:
- the one `RuntimeClient`;
- the one `SessionOwner`;
- transient selected-proxy projection;
- session invalidation when that projection changes.

Product Settings continues to own:
- durable proxy mode/list/selected/rotation/IPv6 state.

No `TelegramWebRuntime`, `TelegramWebProvider`, parallel settings store, or second session owner is permitted.

## 7. Failure and resource bounds

R20. Carrier input/output is bounded. Per-stream pending bytes/items, global pending bytes/items, local browser pending bytes, control-frame reserve, Welcome timeout, write-progress timeout, and retry/adoption fences follow the fixed-U intent and fail closed rather than silently bypassing the selected proxy.

R21. Enabled Web selection MUST NOT fall back to direct TCP when the browser carrier is absent, malformed, timed out, or lost.

R22. Cancellation/deadline continues through the existing `NetworkAttemptControl`; cancelling an attempt shuts down the local logical stream and cannot switch to direct transport.

## 8. Stage-A contracts (authored, not run)

Required contract vectors:
1. `Open(1,[])` -> `01 00 00 01 00 00 00 00`.
2. `Data(0x00ff01,[aa,bb])` -> `02 00 ff 01 00 00 00 02 aa bb`.
3. `Window(0x123456,0x01020304)` -> `04 12 34 56 00 00 00 04 01 02 03 04`.
4. incomplete frames remain buffered; unknown type, >1 MiB payload, >4096-frame batch fail closed.
5. no non-Welcome frame before Welcome.
6. Data consumes receive credit and only local reads return it.
7. Window is exactly four bytes/nonzero; send credit saturates.
8. late Data/Window/Close for a tombstoned stream is tolerated; never-known stream fails.
9. carrier loss closes every stream and clears generation-local state.
10. Web selection requires port 443 and canonical bridge capability inputs.
11. selected Web never falls back to direct transport.
12. bootstrap URL/token are not exposed by ordinary runtime status.

All contracts remain `validation_status=not-run` in Stage A.

## 9. Acceptance criteria

AC-1. Relay stream/window/bootstrap code is reachable from `NetworkProxyKind::Web` on the existing shipping session call graph.
AC-2. MTProto obfuscation/framing remains reused above the logical stream.
AC-3. No second runtime/provider/settings/session owner exists.
AC-4. Web selection fails closed if its carrier cannot be established.
AC-5. Product Settings still publishes only the transient selected projection; secrets are absent from status/reporting.
AC-6. Contracts are present but not executed.
AC-7. Desktop unique parity-ledger/P0/source-closure/evidence records `implemented / validation_status=not-run / accepted=false`.
AC-8. Stage-B executable verification remains pending.

## 10. Provenance

Fixed U:
- `Telegram/SourceFiles/mtproto/web_proxy/web_proxy_frame.h@a5ba491fed67d70aee88e6ec2e3ac47c991e78f3`
- `Telegram/SourceFiles/mtproto/web_proxy/web_proxy_frame.cpp@bd1d7b310f40203f9da914efc8bdd6711035884b`
- `Telegram/SourceFiles/mtproto/web_proxy/web_proxy_transport.h@f58850d2bc2ceabd93113d7725903e1c0d92d71b`
- `Telegram/SourceFiles/mtproto/web_proxy/web_proxy_transport.cpp@e25ad970e84686e9aca061d02c274048793b5ad4`
- `Telegram/SourceFiles/mtproto/web_proxy/web_proxy_webview.cpp@8a2efc8fb15e400366f199cd956effc8ad362da8`
- `Telegram/SourceFiles/mtproto/details/mtproto_web_proxy_socket.cpp@9aa05e7034031310de4aa9d410fbb15d21c63ee8`
- `Telegram/SourceFiles/mtproto/connection_tcp.cpp@d721a9058e1885d2964ded55bd4e4c7c2ce9e381`
- `Telegram/SourceFiles/mtproto/mtproto_proxy_data.cpp@f9748fcb6358dd8ce7429536ec2b999cb2a989ef`

Live owner anchors before implementation:
- `native/telegram-network/src/lib.rs@cf1247b3ec2486ce5e05f5a08ceb30b1e460e1db`
- `native/telegram-runtime/src/runtime.rs@b605b13c110b545d0c703b7fae1cc27d3d10afab`
- `native/telegram-protocol/src/web_proxy.rs@98e65d426242f9ff1ca0a3a2213dae1e41eca824`

## 11. Spec-compliance disposition

The previous production blocker (“no selectable Web carrier call path”) is closed by the current source-neutral selected-proxy projection. Implementation is authorized only in the existing owners above. Stage-A source review may establish `implemented`, but `verified`, `accepted`, candidate readiness, packaging and release remain forbidden until Stage B executable validation.
