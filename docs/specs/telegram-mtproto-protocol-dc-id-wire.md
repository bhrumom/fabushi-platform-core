# Telegram MTProto protocol DC-id wire derivation — Specification

Status: active  
Owner: native/telegram-protocol  
Last updated: 2026-11-01  
Related task: TDRP-R9-MTPROTO-PROTOCOL-DC-ID-WIRE-001 / 7457eb1c-11a3-4626-8563-a3e495872536 round 76

## 1. Context / problem

The existing direct-TCP obfuscated connection-start implementation accepts the signed 16-bit protocol DC id that is written into bytes 60..62 of the 64-byte prefix, but the shipping network carrier currently derives only the simple main/test case. Fixed-U Telegram Desktop has additional source-level semantics in SessionPrivate::getProtocolDcId(): shifted IDs are reduced to their bare DC identity, temporary identities are normalized for the wire, test mode adds 10000, and MediaCluster connections negate the final wire id.

These transformations are wire identity rules. They must not become a second scheduler, endpoint registry, auth-key lifecycle, or Telegram runtime.

## 2. Goal

Close the exact fixed-U media-cluster negative protocol-DC-id and temporary-DC normalization responsibility inside the existing shared telegram-protocol DC owner, then have the existing telegram-network socket carrier consume that result without taking ownership of the wire rule.

## 3. Non-goals / out of scope

- Proxy/Web/TLS camouflage, TLS ClientHello/TlsBlock.
- Connection racing, retry, reconnect, backoff, cancellation, endpoint-order policy, or quick-ACK lifecycle.
- Auth-key binding, persistence, expiry, renewal, or the TemporaryMediaCluster key-creation lifecycle.
- Arbitrary request histories, gzip, complete difference/differenceSlice decoding/application.
- Already closed connection-start secret/nonce/AES-CTR, framing, envelope, secure inner message, or handshake responsibilities.
- Reproducing Telegram Desktop ShiftedDcId scheduling APIs or creating a second DC/session owner.

## 4. Requirements

- R1: The pure protocol derivation must use fixed-U kDcShift=10000 semantics: directory/bare DC id is shifted_dc_id % 10000.
- R2: The protocol wire's simple DC id must normalize any bare id >= 1000 by subtracting the fixed-U temporary threshold 1000 exactly once; lower bare ids remain unchanged.
- R3: Test mode must add the fixed-U test shift 10000 to the normalized simple DC id before media sign handling.
- R4: MediaCluster wire role must negate the tested DC id; Regular wire role must preserve it. CDN is not implicitly MediaCluster.
- R5: The emitted wire id must be represented as signed i16 and fail closed if the source-neutral Rust input is outside the representable/valid shifted-DC domain rather than silently narrowing.
- R6: The derivation must return both the directory lookup id (bare identity, preserving temporary ids such as 1002) and the signed wire id. Endpoint lookup and wire identity must remain separate.
- R7: Existing telegram-network::connect_directory must delegate this derivation to telegram-protocol, use the returned directory id for DcDirectory candidate lookup, and pass the returned wire id to the existing ObfuscatedConnectionStart. No new socket/scheduler/runtime owner is allowed.
- R8: The current source-neutral network purpose mapping may map DcPurpose::Media to MediaCluster wire role and Main/Cdn to Regular. This mapping is only an input classification for the pure wire rule; broader Telegram Desktop key-selection transitions remain excluded.
- R9: Existing simple production/test main-DC behavior must remain unchanged for positive unshifted DC ids.
- R10: Stage A must author focused contracts but execute none. validation_status remains not-run and accepted remains false.

## 5. Current state

At platform-core checkpoint 5b7ed34df4c8063fe3db9627552bc673dda9c92f:
- native/telegram-protocol/src/dc.rs owns DcDirectory, DcEndpoint, DcPurpose and related source-neutral DC routing types.
- native/telegram-network/src/lib.rs owns sockets and has a private protocol_dc_id() that only applies the 10000 test shift to the caller's dc_id before i16 conversion.
- native/telegram-protocol/src/transport.rs already owns the obfuscated prefix and writes the caller-supplied i16 protocol DC id into the encrypted tail.
- No existing protocol helper carries fixed-U shifted/media/temporary protocol-DC semantics.

## 6. Target state

native/telegram-protocol/src/dc.rs is the one source of truth for pure protocol DC-id derivation. native/telegram-network/src/lib.rs consumes that helper while retaining socket and endpoint-attempt ownership. The desktop provider mirror is synchronized after canonical implementation.

## 7. Architecture and ownership boundaries

Canonical wire/DC owner: native/telegram-protocol/src/dc.rs.
Canonical socket carrier: native/telegram-network/src/lib.rs.
Existing obfuscated prefix owner: native/telegram-protocol/src/transport.rs.

The protocol owner may expose a small source-neutral role enum and a derived value containing directory_dc_id plus wire_id. It must not own endpoint iteration, retries, authentication, persistence, or session scheduling.

## 8. Interfaces / contracts / schemas / data flow

Input:
- shifted_dc_id: i32
- test_mode: bool
- wire role: Regular | MediaCluster

Pure transform:
shifted_dc_id
-> bare/directory id = shifted_dc_id % 10000
-> temporary normalization for wire when bare >= 1000
-> optional +10000 test shift
-> optional MediaCluster negation
-> signed i16 wire id

Network flow:
connect_directory input -> protocol derivation -> DcDirectory::candidates(directory_dc_id, purpose, ...) -> existing TcpTransport::connect_obfuscated(..., wire_id, ...).

## 9. Constraints and non-functional requirements

- Source-neutral Rust only; no retained Telegram C++, FFI, sidecar, subprocess, or second provider/runtime.
- Existing-owner-first.
- Fail closed on invalid source-neutral shifted DC ids.
- No CI, build, typecheck, lint, schema validator, generator, test execution, package, deployment, installation, runtime acceptance, Actions/status/check query, or CI trigger during this Stage-A slice.

## 10. Failure modes and edge cases

- shifted_dc_id with bare id 0 is invalid for the current Fabushi DC directory and must fail closed.
- Temporary 1002 must use directory id 1002 but wire id 2 (or 10002 in test mode).
- Shifted media 160002 must use directory id 2 but wire id -2 (or -10002 in test mode).
- Shifted temporary media 161002 must preserve directory id 1002 while normalizing wire id to -2/-10002.
- DcPurpose::Cdn must not receive the media negative sign merely because file-oriented transport behavior may also be used.
- No helper may mutate directory, session, key, retry, or request-history state.

## 11. Implementation strategy

1. Add the pure role/result/error-safe derivation to native/telegram-protocol/src/dc.rs.
2. Re-export it through the existing protocol facade.
3. Replace telegram-network's local main/test-only helper with delegation to the protocol owner and separate directory-id vs wire-id use.
4. Author protocol and network contracts without running them.
5. Synchronize the canonical touched provider files into the desktop checkpoint and update the one parity ledger/P0/source-closure/evidence chain.

## 12. Verification / test strategy

Authored-only Stage-A contracts must cover:
- regular production and test ids;
- temporary production and test normalization;
- media-cluster negative production and test ids;
- shifted media bare-id reduction;
- shifted temporary-media combination;
- bare-id-zero rejection;
- network directory lookup using the returned bare id while the prefix receives the derived signed wire id;
- Main/Cdn regular-sign preservation.

No contract is executed in Stage A.

## 13. Acceptance criteria / Definition of Done

- AC-1: Canonical protocol owner contains the exact pure fixed-U derivation and no scheduling/auth lifecycle.
- AC-2: Existing network carrier delegates wire-id derivation and keeps endpoint attempts/sockets as its only responsibility.
- AC-3: Focused contracts are authored and explicitly not run.
- AC-4: Desktop provider mirrors and the unique ledger/P0/source-closure evidence are synchronized through non-force expected-SHA CAS updates.
- AC-5: validation_status=not-run and accepted=false remain explicit.

## 14. Release / migration / rollback

Stage A only. No release or deployment. Rollback is a normal non-force revert of the isolated checkpoint commits before Stage-B candidate freeze.

## 15. Observability / evidence

Source-level evidence is Git blob/commit identity plus authored contract paths. Executable evidence is intentionally absent until Stage B.

## 16. References / provenance

Fixed source authority: telegramdesktop/tdesktop@f985723b552b30fe1c17d377d57f44e61a15a8c4.

- Telegram/SourceFiles/mtproto/session_private.cpp@13cbc76ec723386027b5b4dcac79999b791c4614 — SessionPrivate::getProtocolDcId.
- Telegram/SourceFiles/mtproto/facade.h@39e2f732c7df32af37ebf8222e887aec7e11b256 — temporary/media helper semantics.
- Telegram/SourceFiles/mtproto/core_types.h@9d025d8ae265e3fe2eef3f32fc609e79a0c4de7f — kDcShift/BareDcId/shift constants.
- Telegram/SourceFiles/mtproto/mtp_instance.h@eeb56d29cc8316bd6600c26b6f8edf3a912601eb — kTemporaryMainDc=1000.
- Telegram/SourceFiles/mtproto/connection_abstract.h@217b684fa222c42e968dfddeaa62f16a0054017f — kTestModeDcIdShift=10000.
- Telegram/SourceFiles/mtproto/mtproto_dc_options.h@4e0b979c4a84661f3365196ab8702dc8cb60fd03 and mtproto_dc_options.cpp@d8499fc1df2ed7fdb88da687a04b824d243ff49a — DcType and MediaCluster classification.
- Telegram/SourceFiles/mtproto/connection_tcp.cpp@d721a9058e1885d2964ded55bd4e4c7c2ce9e381 — existing consumer of the signed protocol DC id in connection-start obfuscation.

## 17. Spec compliance record

| Requirement / AC | Status | Evidence / reason |
| --- | --- | --- |
| R1-R10 | pending | Stage-A implementation not yet recorded. |
| AC-1 | pending | |
| AC-2 | pending | |
| AC-3 | pending | |
| AC-4 | pending | |
| AC-5 | pending | |
