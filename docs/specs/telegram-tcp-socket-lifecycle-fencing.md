# Telegram TCP socket lifecycle fencing — Specification

Status: active  
Owner: canonical shared telegram-network / existing RuntimeClient SessionOwner  
Last updated: 2026-11-01  
Related task: TDRP-001 / 7457eb1c-11a3-4626-8563-a3e495872536 round 104

## 1. Context / problem

Fixed-U `telegramdesktop/tdesktop@f985723b552b30fe1c17d377d57f44e61a15a8c4` separates a TCP socket's connect-only lifetime from the adopted connection/session lifetime. `mtproto_tcp_socket.cpp@b3130fd706b7cbfa33f7c9f54ac5e87abe9c3e10` applies `QTcpSocket::setProxy` before connect, forwards connected/disconnected/readyRead/error signals through the socket owner, and collapses detailed socket errors into the connection error lifecycle after logging. `connection_tcp.cpp@d721a9058e1885d2964ded55bd4e4c7c2ce9e381` destroys its connect-only lifetime after the connection reaches Ready and destroys all socket lifetimes when finished. `connection_abstract.cpp@3baabf3f72c7c7edba3151814681fbf167a5ca0a` disconnects old callbacks before disposing a connection. `session_private.cpp@13cbc76ec723386027b5b4dcac79999b791c4614` removes callbacks by exact connection identity, adopts one winning connection, and restarts only from the current owned connection set.

The existing Rust `NetworkAttemptControl` correctly cancels an active socket and bounds connect/handshake/request I/O, but controlled connect/resume paths currently return an adopted `EstablishedSession` whose `TcpTransport` still retains the connect attempt control. That lets the connect-stage deadline/ownership leak into later session I/O until a later request happens to replace/clear it.

## 2. Goal

Close the exact connect/error/socket-lifecycle handoff responsibility inside the existing telegram-network owner: connect/handshake attempts remain cancellable while active, but a successfully adopted live session is detached from the connect-only attempt before it is returned. Later request-scoped controls may attach temporarily and are cleared when that request settles.

## 3. Non-goals / out of scope

- No new runtime, provider, settings, session, transport, retry, or persistence owner.
- No second proxy implementation and no direct fallback when an enabled proxy fails.
- No change to ProductSettingsOwner or the RuntimeClient network-proxy projection schema.
- No broad Telegram Desktop connection racing/retry scheduler implementation.
- No claim that fixed-U `protocolForFiles` OS send/receive socket-buffer sizing is closed by this slice; that adjacent file-transfer tuning remains separately open.
- No executable validation in Stage A.

## 4. Requirements

- R1: A controlled TCP/proxy connect or auth-key handshake remains owned by its `NetworkAttemptControl` until the connect/handshake operation has successfully completed.
- R2: Before a successfully controlled `EstablishedSession` is handed to RuntimeClient, telegram-network must clear the connect-only control from the transport and clear the control's active-stream registration without cancelling the now-adopted connection.
- R3: A later `EstablishedSession::invoke_raw_controlled` must be able to attach the request's control to that live transport and clear it when the invocation settles.
- R4: Send flushing while a request/attempt control is attached must use the same bounded write/cancellation path as the preceding writes; it must not bypass the control or recurse.
- R5: Connect, resolver, proxy handshake, auth handshake, read, write, disconnect, and generic socket failure remain typed failures in the existing telegram-network/session path; enabled-proxy failures never fall back to direct transport.
- R6: RuntimeClient `SessionOwner::invalidate_network_transport` remains the projection-change fence: it cancels owned requests and drops live sessions before the new transient proxy projection becomes authoritative.
- R7: No proxy credentials, socket handles, or internal control state may be added to public status/error projections.

## 5. Current state

`NetworkAttemptControl::register_stream` owns a cloned TCP stream so cancellation can shut down the current operation. Controlled connection constructors pass that control into `TcpTransport`, and `TcpTransport::apply_read_bound/apply_write_bound` consult its absolute deadline. The transport is currently returned with that control still installed. `invoke_raw_controlled` already has the desired request-local attach/clear pattern. RuntimeClient already cancels owned requests and clears live sessions on carrier-projection changes.

## 6. Target state

Controlled connect/handshake success performs an explicit ownership handoff: the live transport remains open, the connect attempt's active-stream clone is released, and the transport no longer carries the connect-only deadline. Request-scoped invocation can subsequently reattach the same request control for the RPC interval and clear it again. Failure paths remain fail-closed and retain cancellation ownership until the caller settles the request.

## 7. Architecture and ownership boundaries

- `native/telegram-network`: sole socket/carrier and `NetworkAttemptControl` lifecycle owner.
- Existing `EstablishedSession`: sole live MTProto session/network object; no parallel socket owner.
- Existing RuntimeClient `SessionOwner`: sole request/session fencing owner above telegram-network.
- ProductSettingsOwner and TelegramClientLease remain unchanged; they only publish/consume the transient selected-proxy projection.

## 8. Interfaces / contracts / schemas / data flow

Controlled request flow:

`SessionOwner begin_request -> NetworkAttemptControl -> controlled connect/resume/auth handshake -> successful EstablishedSession -> clear connect-attempt registration -> invoke_raw_controlled attaches request control -> bounded send/flush/read -> clear request control -> SessionOwner finish/cancel`.

Projection mutation remains:

`ProductSettingsOwner -> telegram.applyNetworkProxy -> RuntimeClient detects changed projection -> invalidate_network_transport -> cancel owned controls/drop live sessions -> replace transient projection`.

No public schema changes are introduced.

## 9. Constraints and non-functional requirements

- Stage A: source editing and authored tests only; no build/lint/test/generator/CI execution.
- Preserve existing-owner-first composition and current protocol/auth/session owners.
- Cancellation must target only the currently owned stream; successful ownership handoff must not itself shut down the adopted stream.
- Keep credentials transient and out of diagnostics/status.

## 10. Failure modes and edge cases

- A stale connect deadline remains attached and later rejects unrelated live-session I/O.
- Clearing a control cancels the adopted socket instead of only releasing connect-attempt ownership.
- Flush bypasses the attempt deadline/cancellation boundary.
- A failed proxy/connect/auth handshake silently retries direct transport.
- Projection changes leave old requests or sessions live.
- File-transfer socket-buffer tuning is accidentally claimed as implemented by this lifecycle-only slice.

## 11. Implementation strategy

1. Reuse `TcpTransport::clear_attempt_control` as the ownership-handoff primitive.
2. On successful controlled direct/proxy establish/resume wrappers, clear the connect-only control before returning the session.
3. Keep handshake failure paths unchanged so the caller can still cancel the owned operation.
4. Correct the bounded flush helper to flush the underlying transport and route `send_payload` through it.
5. Add focused source contracts in the existing telegram-network test module; do not execute them in Stage A.
6. Mirror canonical network/spec bytes into the existing desktop provider path.

## 12. Verification / test strategy

Authored-only Stage-A contracts:
- controlled resumed session releases its connect-attempt transport ownership while keeping the control uncancelled;
- request-scoped attach/clear can re-own and release the same live transport;
- zero/expired control is observed only while attached, proving the adopted transport is not permanently poisoned by a stale connect deadline;
- existing proxy fail-closed and RuntimeClient projection-fencing contracts remain applicable.

Executable verification is deferred to the Stage-B GitHub Actions campaign.

## 13. Acceptance criteria / Definition of Done

- AC-1: Canonical telegram-network controlled establish/resume paths detach connect-attempt control on success.
- AC-2: Bounded flush delegates to the underlying stream and `send_payload` uses it.
- AC-3: Focused lifecycle contracts are authored in the existing test owner and remain not-run in Stage A.
- AC-4: Desktop provider mirror is byte-identical to canonical changed source/spec.
- AC-5: Unique parity ledger/P0/source-closure/evidence records the exact fixed-U responsibility as implemented, `validation_status=not-run`, `accepted=false`.
- AC-6: RuntimeClient fencing and settings ownership remain unchanged.
- AC-7: Fixed-U `protocolForFiles` buffer sizing remains explicitly open rather than being silently claimed.

## 14. Release / migration / rollback

No data migration. Stage-A checkpoint commits are reversible by ordinary Git history. Release/packaging is not applicable until Stage B/C.

## 15. Observability / evidence

Source-level evidence records fixed-U blob identities, canonical/desktop changed blob identities, authored contract locations, and CAS checkpoint commits. No runtime pass claim is permitted before Stage B.

## 16. References / provenance

- fixed-U `Telegram/SourceFiles/mtproto/details/mtproto_tcp_socket.cpp@b3130fd706b7cbfa33f7c9f54ac5e87abe9c3e10`
- fixed-U `Telegram/SourceFiles/mtproto/details/mtproto_abstract_socket.cpp@653033365b543e554104c99a963ee194ddb830d9`
- fixed-U `Telegram/SourceFiles/mtproto/connection_abstract.cpp@3baabf3f72c7c7edba3151814681fbf167a5ca0a`
- fixed-U `Telegram/SourceFiles/mtproto/connection_tcp.cpp@d721a9058e1885d2964ded55bd4e4c7c2ce9e381`
- fixed-U `Telegram/SourceFiles/mtproto/session_private.cpp@13cbc76ec723386027b5b4dcac79999b791c4614`
- existing `native/telegram-network/src/lib.rs`
- existing `native/telegram-runtime/src/runtime.rs`

## 17. Spec compliance record

Source-level compliance was reviewed at the canonical production commit `dc16f388756d681ef4ee6683b98525edef869e1f`, with desktop provider mirror/bookkeeping checkpoint `734e37f377e7fe58b7a646a6b64b29a92bf9629e`. This is Stage-A source evidence only; executable verification remains deferred to Stage B.

| Requirement / AC | Status | Evidence / reason |
| --- | --- | --- |
| R1 | passed | Existing `NetworkAttemptControl::register_stream/cancel` retains exact active socket ownership through controlled connect/proxy/auth-handshake. |
| R2 | passed | `resume_auth_key_controlled_with_proxy` and `establish_auth_key_controlled_with_proxy` now call `clear_attempt_control` only after successful establishment, releasing active-stream registration without cancelling the adopted connection. |
| R3 | passed | Existing `EstablishedSession::invoke_raw_controlled` attaches the owned request control, invokes the RPC, then clears it; focused re-ownership contract authored. |
| R4 | passed | `flush_bounded` now calls the underlying stream flush after applying the write bound, and `send_payload` delegates final flush to `flush_bounded`. |
| R5 | passed | Existing direct/proxy connect and handshake paths return typed `NetworkError`; enabled proxy carrier branches remain fail-closed and do not enter the disabled/direct branch on error. Executable fault validation is still not-run. |
| R6 | passed | RuntimeClient `SessionOwner::invalidate_network_transport` remains unchanged: it cancels owned controls and clears live sessions before `apply_network_proxy_command` replaces the projection. |
| R7 | passed | Runtime status continues to expose only enabled/type/route metadata; proxy credentials, sockets and control internals are not projected. |
| AC-1 | passed | Canonical network blob `63f9c4a26385a776fbdd42af3d0de44a37857691`. |
| AC-2 | passed | Same canonical network blob contains bounded underlying flush and `send_payload -> flush_bounded`. |
| AC-3 | passed | Two lifecycle contracts are authored in the existing telegram-network test module; per Stage A they were not executed. |
| AC-4 | passed | Desktop provider network mirror at checkpoint `e4fe918b8e8ba5589d1ab3d2cdae888087933719` is byte-identical at blob `63f9c4a26385a776fbdd42af3d0de44a37857691`; desktop spec mirror is synchronized by the round-104 checkpoint line. |
| AC-5 | passed | Unique ledger/P0/source-closure/evidence checkpoint `734e37f377e7fe58b7a646a6b64b29a92bf9629e` records `implemented / validation_status=not-run / accepted=false`, responsibilities=123, unknown=16172, unread=16145. |
| AC-6 | passed | RuntimeClient fencing source remains unchanged at blob `3a3da0eb4bf004569a3989e05e55c8b86a199c41`; no settings/runtime/session owner was added. |
| AC-7 | passed | `protocolForFiles` SendBuffer/ReceiveBuffer socket-option tuning is explicitly excluded from this slice and remains open in round135 evidence. |

Stage-B build/unit/contract/integration/packaged validation remains `not-run`; this source-level compliance record does not mark the responsibility verified or accepted.
