# Telegram file-transfer socket-buffer tuning — Specification

Status: active  
Owner: canonical shared telegram-network  
Last updated: 2026-11-01  
Related task: TDRP-001 / 7457eb1c-11a3-4626-8563-a3e495872536 round 105

## 1. Context

Fixed-U `telegramdesktop/tdesktop@f985723b552b30fe1c17d377d57f44e61a15a8c4` defines `kFilesSendBufferSize` and `kFilesReceiveBufferSize` as 2 MiB in `mtproto_abstract_socket.h@4283a353dd00db4b12044ef5a1d46cfbde7f659b`. Both `mtproto_tcp_socket.cpp@b3130fd706b7cbfa33f7c9f54ac5e87abe9c3e10` and `mtproto_tls_socket.cpp@e7562e3ca1a1da03f5d368085d5e61d887ab3f16` request those OS send/receive buffer sizes when `protocolForFiles` is true.

The actual consumer is narrower than all file sessions. `session_private.cpp@13cbc76ec723386027b5b4dcac79999b791c4614` computes `protocolForFiles = isMediaClusterDcId(_shiftedDcId) || (_realDcType == DcType::Cdn)`; the adjacent `isUploadDcId(_shiftedDcId)` predicate is deliberately commented out. `facade.h@39e2f732c7df32af37ebf8222e887aec7e11b256` defines MediaCluster to include download, group-call stream, export-media, and updater shifts. `mtp_instance.cpp@d25fe3b72298fab371854f80186ee8e4777ea087` puts both download and upload shifted sessions on file-session threads, so file-thread placement is not the tuning predicate. `file_upload.cpp@a19470a24b6a62b58c63d8cc21d831641d970260` uses `uploadDcId`; `download_manager_mtproto.cpp@d0007eb17fb0bcfbdffd89b86d8efdb900fcd363` uses `downloadDcId`.

Current Fabushi already has the needed source-neutral split: `telegram-network` owns native sockets/carriers, `telegram-protocol::DcPurpose` distinguishes Main/Media/Cdn, and `telegram-media` owns scheduling/progress/retry/integrity while adapters perform I/O.

## 2. Goal

Within the existing telegram-network owner:
- request a 2 MiB send buffer and 2 MiB receive buffer for native TCP sockets used by `DcPurpose::Media` or `DcPurpose::Cdn`;
- leave `DcPurpose::Main` on the default OS buffer profile;
- apply tuning before proxy negotiation, TLS camouflage, connection-start bytes, or framed MTProto I/O;
- preserve the same profile for direct, SOCKS5/HTTP forward-proxy, and MTProto-proxy native TCP carriers.

## 3. Non-goals

- No new file/media/network/runtime/provider/settings/session owner.
- No change to telegram-media scheduling, chunk sizing, persistence, retry, or integrity.
- Do not infer tuning for upload-shift sessions; fixed-U explicitly excludes that predicate.
- No WebProxy relay/browser window tuning in this slice.
- No new dependency solely for `setsockopt`.
- Do not require kernel-reported effective buffer size to equal the requested bytes.
- No executable validation in Stage A.

## 4. Requirements

- **R1:** retain distinct send/receive constants of exactly 2 MiB with fixed-U provenance.
- **R2:** Media and Cdn select the file-transfer socket profile; Main selects default.
- **R3:** direct native TCP applies the profile after connect and before protocol/TLS bytes.
- **R4:** SOCKS5/HTTP and MTProto-proxy native TCP apply the same profile before proxy/TLS/protocol bytes.
- **R5:** socket-option failure propagates through existing typed `NetworkError::Io`; no silent untuned/direct fallback.
- **R6:** telegram-network remains socket/carrier owner; telegram-media remains transfer-policy owner; RuntimeClient remains session/request owner.
- **R7:** supported native families have explicit adapters: Linux/Android, Apple/BSD family, and Windows; unsupported Unix families fail closed instead of pretending tuning occurred.
- **R8:** source contracts cover exact values and Main vs Media/Cdn predicate, authored but not run in Stage A.

## 5. Target data flow

`DcPurpose -> NativeSocketBufferProfile -> connect_directory_with_projection -> selected native TcpStream constructor -> OS send/receive option requests -> proxy/TLS/MTProto I/O`.

The profile is private to telegram-network. Existing generic public transport constructors remain default-profile unless reached through the purpose-aware directory composition.

## 6. Failure and edge cases

- If either option write fails, abort that connection attempt with `NetworkError::Io`.
- With an enabled proxy, tune the actual proxy TCP socket and preserve fail-closed proxy semantics.
- WebProxy uses a separate relay stream and is not claimed as native socket-buffer equivalence here.
- Main/auth/config sessions remain default-profile.
- Upload shifted-session/file-thread membership alone does not activate tuning.
- OS normalization of effective buffer accounting does not change the source contract: the requested value is 2 MiB.

## 7. Implementation strategy

1. Add private fixed-U send/receive constants plus a private `NativeSocketBufferProfile`.
2. Derive the profile once from `DcPurpose` in `connect_directory_with_projection`.
3. Pass it to all native TCP constructors selected there.
4. Apply both socket options immediately after `TcpStream::connect_timeout`.
5. Keep generic public constructors default-profile.
6. Add authored-only contracts; do not execute them in Stage A.

The standard library does not expose TCP send/receive buffer setters, so use the smallest private platform adapter around `setsockopt`, with no new dependency and no lifecycle ownership.

## 8. Verification

Stage A: manual source review plus authored contracts only. Do not run build, lint, tests, generators, dependency resolution, schema validation, CI, packaging, or runtime acceptance.

Stage B after full implementation closure: compile supported targets, execute contracts, and obtain platform integration evidence for direct/proxy media/CDN paths and typed option failures.

## 9. Acceptance criteria

- **AC1:** canonical telegram-network represents exact 2 MiB send/receive requests.
- **AC2:** Media/Cdn are tuned and Main is not, with authored contracts.
- **AC3:** direct, SOCKS5/HTTP, and MTProto-proxy native carriers receive the purpose-derived profile before protocol I/O.
- **AC4:** option failure maps to `NetworkError::Io`.
- **AC5:** no parallel owner is created.
- **AC6:** desktop mirror and unique ledger/P0/source-closure/evidence record `implemented / validation_status=not-run / accepted=false`.
- **AC7:** executable validation remains deferred.

## 10. Provenance

- `telegramdesktop/tdesktop@f985723b552b30fe1c17d377d57f44e61a15a8c4`
- `mtproto_abstract_socket.h@4283a353dd00db4b12044ef5a1d46cfbde7f659b`
- `mtproto_tcp_socket.cpp@b3130fd706b7cbfa33f7c9f54ac5e87abe9c3e10`
- `mtproto_tls_socket.cpp@e7562e3ca1a1da03f5d368085d5e61d887ab3f16`
- `session_private.cpp@13cbc76ec723386027b5b4dcac79999b791c4614`
- `mtp_instance.cpp@d25fe3b72298fab371854f80186ee8e4777ea087`
- `facade.h@39e2f732c7df32af37ebf8222e887aec7e11b256`
- `file_upload.cpp@a19470a24b6a62b58c63d8cc21d831641d970260`
- `download_manager_mtproto.cpp@d0007eb17fb0bcfbdffd89b86d8efdb900fcd363`
- existing `native/telegram-network/src/lib.rs`
- existing `native/telegram-media/src/lib.rs`
- existing `native/telegram-protocol/src/dc.rs`

## 11. Spec compliance record

Source-level implementation is present in canonical telegram-network blob `6dece8da8f9a1512dcd17bc76fd1cc9ce43c1ef6`. This record is Stage-A source review only; executable verification remains intentionally not run.

| Requirement / AC | Status | Source evidence |
| --- | --- | --- |
| R1 / AC1 | passed | Distinct `FILES_SEND_BUFFER_BYTES` and `FILES_RECEIVE_BUFFER_BYTES` are both exactly `2 * 1024 * 1024`. |
| R2 / AC2 | passed | `socket_buffer_profile_for_purpose` maps Main->Default and Media/Cdn->Files; two contracts are authored. |
| R3 / AC3 | passed | Direct directory connection passes the derived profile into the native obfuscated TCP constructor, which applies it immediately after connect. |
| R4 / AC3 | passed | SOCKS5/HTTP forward-proxy and MTProto-proxy native constructors receive the same derived profile before proxy/TLS/protocol bytes. |
| R5 / AC4 | passed | Native option adapters return `io::Result`; `?` promotes failures through existing `NetworkError::Io`. |
| R6 / AC5 | passed | Only telegram-network is changed; telegram-media and RuntimeClient owners are unchanged. |
| R7 | passed (source-level) | Linux/Android, Apple/BSD-family, and Windows adapters are explicit; other Unix/native targets fail closed. Compile/runtime evidence is deferred. |
| R8 | passed | Two source contracts are authored in the existing test module and were not executed. |
| AC6 | passed | Desktop production/spec mirror is `23d73b32e301408f9c8aed151cb1f9e442516e13`; unique ledger/P0/source-closure/round136 evidence checkpoint is `77e9dc1a45e9486ea8cf0ebe912b1a15c83c17a7`, with `implemented / validation_status=not-run / accepted=false` and responsibilities_total=124. |
| AC7 | passed | No executable validation was run in Stage A. |

The responsibility remains `implemented / validation_status=not-run / accepted=false` until Stage-B executable evidence exists. Source-level R1-R8 / AC1-AC7 compliance is complete; this does not constitute executable verification.
