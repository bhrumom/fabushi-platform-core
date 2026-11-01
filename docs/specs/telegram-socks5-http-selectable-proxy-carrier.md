# Telegram SOCKS5 / HTTP selectable proxy carrier

Status: Stage A implementation spec
Fixed U: telegramdesktop/tdesktop@f985723b552b30fe1c17d377d57f44e61a15a8c4
Validation: not-run

## Goal

Close the remaining fail-closed SOCKS5 and HTTP selected-proxy carrier responsibilities without creating a new runtime, provider, settings owner, session owner, or persistence root.

## Fixed-U behavior

`connection_tcp.cpp` keeps the Telegram DC address/port for SOCKS5 and HTTP, passes the selected proxy to the same TCP socket, and then reuses the ordinary MTProto obfuscated/TLS/framing path. `mtproto_proxy_data.cpp` maps SOCKS5/HTTP to the native proxy carrier with host/port/user/password. `ProxyData::tryCustomResolve` applies custom direct-DoH proxy-host resolution to SOCKS5 (and MTProto) but not HTTP.

## Ownership

- ProductSettingsOwner remains the only durable proxy settings owner.
- RuntimeClient keeps the transient projection and existing session/request fencing.
- telegram-network remains the single socket/carrier lifecycle owner.
- Existing protocol/session/auth-key owners remain unchanged after tunnel establishment.

## Requirements

- TDRP-PROXY-CARRIER-01: Enabled SOCKS5 negotiates RFC1928 CONNECT to the chosen Telegram DC endpoint and supports no-auth plus RFC1929 username/password.
- TDRP-PROXY-CARRIER-02: SOCKS5 proxy hostnames reuse the existing transient direct-NoProxy DoH cache and successful-IP promotion.
- TDRP-PROXY-CARRIER-03: Enabled HTTP negotiates HTTP CONNECT to the chosen Telegram DC endpoint and supports optional Basic proxy authentication.
- TDRP-PROXY-CARRIER-04: HTTP proxy hostname resolution uses the ordinary socket resolver rather than the custom proxy DoH cache.
- TDRP-PROXY-CARRIER-05: Proxy failure never falls back to direct Telegram transport.
- TDRP-PROXY-CARRIER-06: Existing DC endpoint secret, protocol DC identity, obfuscation/TLS/framing and auth/session lifecycle are reused after the tunnel is ready.
- TDRP-PROXY-CARRIER-07: NetworkAttemptControl continues to bound connect, negotiation and later I/O.
- TDRP-PROXY-CARRIER-08: Credentials remain transient and are excluded from result/error/debug surfaces.

## Failure behavior

Missing/invalid selection, resolver failure, unsupported SOCKS method, authentication failure, non-200 HTTP response, malformed proxy reply, cancellation, timeout or tunnel failure is an error and must not create a direct connection.

## Verification

Source contracts are authored in the existing telegram-network test surface for SOCKS5 CONNECT framing, HTTP CONNECT/auth request shape, fail-closed behavior and existing RuntimeClient session fencing. Stage A does not execute them.

## Acceptance

Stage-A implementation may be marked implemented only when both carriers are reachable through the existing RuntimeClient projection and telegram-network composition, contracts are authored, and ledger/evidence remain validation_status=not-run and accepted=false.
