# Telegram Special Config Payload Boundary — Specification

Status: active  
Owner: native/telegram-protocol  
Last updated: 2026-11-01  
Related issue/task/PR: Fabushi migration task 7457eb1c-11a3-4626-8563-a3e495872536 round 84

## 1. Context / problem

Fixed upstream remains `telegramdesktop/tdesktop@f985723b552b30fe1c17d377d57f44e61a15a8c4`.
Round 64 already closed the exact TL wire family `ipPort` / `ipPortSecret` /
`accessPointRule` / `help.configSimple` in the existing
`fabushi-telegram-protocol` owner, while intentionally leaving special-config
web acquisition, decryption, expiry/prefix policy and DC endpoint application
fail-closed. Round 114 additionally confirmed that enabling a proxy suppresses
special-config acquisition, but that suppression is only one gate around a
larger source responsibility.

The fixed-U call chain is:

1. `ConfigLoader::refreshSpecialLoader()` creates `SpecialConfigRequest` only
   when proxy use and keys-destroyer mode do not suppress it.
2. `SpecialConfigRequest` races direct no-proxy HTTPS acquisition attempts,
   currently Google DNS-over-HTTPS, Mozilla/Cloudflare DNS-over-HTTPS and
   Firestore variants, then converts successful responses into the compact
   special-config byte string.
3. `decryptSimpleConfig()` normalizes the base64 text, performs pinned RSA
   public-key no-padding recovery, AES-256-CBC decryption and SHA-256 digest
   validation, then parses a length-delimited `help.configSimple`.
4. `handleResponse()` rejects expired/empty rules, applies the exact phone
   prefix rules, converts raw IPv4 values, and emits endpoint candidates.
5. `ConfigLoader::sendSpecialRequest()` maps the source DC id to a temporary
   DC id, inserts one temporary TCP-only endpoint into `DcOptions`, sends
   `help.getConfig` through that temporary session, and on success applies only
   the returned `dc_options` through `DcOptions::setFromList()`.
6. `DcOptions::setFromList()` replaces the directory and emits changed-DC
   notifications; temporary ids are excluded from serialized DC options.

Only steps 3-4 are carrier/scheduler independent and can be owned precisely by
the existing protocol owner now.

## 2. Goal

Implement the fixed-U special-config **payload decode and endpoint-selection**
responsibility in the existing source-neutral `fabushi-telegram-protocol`
owner, without creating a second bootstrap runtime, settings owner, network
scheduler, or DC directory owner.

## 3. Non-goals / out of scope

- HTTP/DoH/Firestore request creation, racing, retry cadence, cancellation or
  host-header camouflage.
- Mutation of the global HTTP-derived Unix time offset from response Date
  headers.
- Proxy settings state or proxy-enabled suppression; that remains tracked by
  the existing proxy responsibilities.
- `ConfigLoader` enumeration timers or main-DC `help.getConfig` scheduling.
- Temporary DC session creation, request cancellation, session kill, timeout or
  connection scheduling.
- Runtime insertion/removal of temporary endpoints or replacement/persistence
  of the shipping DC directory.
- Reopening the already implemented `help.configSimple` TL wire parser.
- Executing any build, test, validator or CI in Stage A.

## 4. Requirements

- **R1 — Fixed source identity.** Preserve semantics from fixed-U
  `config_loader.cpp@5d07404ba07223016c6728b986dcba10628d7194`,
  `special_config_request.cpp@fdb65506b0a886bc2daeccb61090bc44be7e2d88`,
  `mtproto_rsa_public_key.cpp@312c8b1ddb3145d5ec3180323e8d72d50c35e526`,
  `mtproto_dc_options.cpp@d8499fc1df2ed7fdb88da687a04b824d243ff49a`,
  `mtproto_domain_resolver.cpp@eeac8f6a8e2df67967ea7b536fb165e2f8d983e9`
  and `facade.h@39e2f732c7df32af37ebf8222e887aec7e11b256`.
- **R2 — Text normalization.** Remove every byte except ASCII
  `A-Z a-z 0-9 + / =`; require exactly 344 remaining base64 bytes and exactly
  256 decoded bytes.
- **R3 — Pinned RSA recovery.** Apply raw 2048-bit RSA public exponentiation
  with the fixed special-config modulus and exponent 65537, equivalent to
  fixed-U `RSA_public_decrypt(..., RSA_NO_PADDING)`, yielding exactly 256
  recovered bytes.
- **R4 — AES-CBC layout.** Use recovered bytes `[0..32]` as the AES-256 key,
  bytes `[16..32]` as the 16-byte IV, and decrypt recovered bytes `[32..256]`
  as fourteen AES-CBC blocks.
- **R5 — Digest.** Split the 224 decrypted bytes into 208 bytes of data plus a
  16-byte digest; require the digest to equal the first 16 bytes of
  SHA-256(data), using constant-time comparison.
- **R6 — Declared TL length.** Interpret data `[0..4]` as little-endian signed
  `realLength`; require `0 < realLength <= 208` and four-byte alignment.
  Parse exactly data `[4..realLength]` with the existing
  `parse_help_config_simple` parser. Padding after `realLength` is not
  semantically inspected.
- **R7 — Freshness/rules gate.** Reject when injected `now_unix > expires`.
  Do not add a new rejection merely because `now_unix < date`; fixed-U does
  not. Reject an empty rules vector.
- **R8 — Phone-prefix matching.** Remove non-digits from the supplied phone.
  Evaluate comma-separated tokens in order: empty token enables a match;
  `+digits` enables a match when the cleaned phone starts with those digits;
  a matching `-digits` rejects the rule immediately; all other tokens have no
  effect. A later matching negative token overrides any earlier positive token.
- **R9 — Endpoint projection.** For each matching rule preserve its `dc_id`,
  `port` and secret bytes exactly. Convert the raw signed TL IPv4 bit pattern
  to the source-equivalent dotted IPv4 address. Do not add port/secret policy
  that fixed-U applies only later in the DC/network path.
- **R10 — Pure boundary.** The implementation must accept payload bytes,
  phone text and `now_unix` as inputs and return typed data/errors only. It
  must not open sockets, schedule retries/timers, read proxy settings, mutate a
  clock, alter a DC directory, or send `help.getConfig`.
- **R11 — Existing-owner-first.** Extend `native/telegram-protocol`; do not
  create a Telegram bootstrap provider/runtime or parallel DC owner.
- **R12 — Dependency discipline.** Reuse the crate's existing `aes`,
  `num-bigint-dig`, `sha2` and `subtle` dependencies. Do not introduce or
  resolve a new cryptography dependency in Stage A.
- **R13 — Deferred validation.** Author source contracts now but do not run
  build, unit, contract, integration, E2E, validators, generators or CI.
- **R14 — Scheduler/application split.** Do not represent the HTTP acquisition,
  temporary-DC `help.getConfig` request, timeout/cancel/kill lifecycle, or
  `DcOptions::setFromList` replacement as implemented until a shipping
  network/runtime owner and caller exist.

## 5. Current state

The existing protocol owner at
`native/telegram-protocol/src/api.rs@ebfb3670b5cd097b9ce288782c7954f5cfe9fc43`
already exposes typed `IpPort`, `AccessPointRule`, `HelpConfigSimple` and
strict TL parsers. Its `Cargo.toml@a62d99f23d370cd75590c7b3186307c1689554ed`
already supplies the cryptographic primitives needed for the pure payload
boundary.

No special-config payload decoder/selector exists yet. The current desktop
shipping/network tree also has no special-config HTTP acquisition or temporary
DC scheduling caller.

## 6. Target state

Add an existing-owner protocol module exporting:

- `SpecialConfigEndpoint { dc_id, ip_address, port, secret }`;
- `SpecialConfigError`;
- `decode_special_config_payload(encoded) -> HelpConfigSimple`;
- `select_special_config_endpoints(config, phone, now_unix) -> Vec<SpecialConfigEndpoint>`;
- `decode_and_select_special_config(encoded, phone, now_unix)` convenience
  composition.

The implementation is deterministic and side-effect free.

## 7. Architecture and ownership boundaries

`telegram-protocol` owns compact payload cryptographic decoding and the
source-defined selection policy because both transform protocol bytes/typed TL
objects without I/O.

A future shipping `telegram-network` / runtime owner may own acquisition and
temporary-DC request scheduling, but it must call this protocol boundary rather
than duplicate cryptographic/prefix semantics.

Canonical DC directory replacement remains with the existing DC/network owner.
The compact payload only produces candidates; it does not mutate the directory.

## 8. Interfaces / contracts / schemas / data flow

`acquired bytes -> protocol decode -> HelpConfigSimple -> selection(phone, now)
-> Vec<SpecialConfigEndpoint>`.

The source callback sentinel `callback(0, "", 0, {})` is represented by normal
function completion rather than a fake endpoint.

The source temporary-id mapping and `f_tcpo_only|f_secret` insertion are not
part of this API.

## 9. Constraints and non-functional requirements

- No network or time side effects.
- No secret logging.
- Fail closed on malformed encoded/encrypted/TL data.
- Constant-time digest comparison.
- No new dependency resolution or lock regeneration in Stage A.
- No executable verification in Stage A.

## 10. Failure modes and edge cases

Typed failures must distinguish at least: normalized encoded length, base64
decode/decoded size, RSA domain/result, encrypted block layout, digest mismatch,
invalid declared length, TL parse error, expiration, and empty rules.

Zero matching phone rules is a successful empty endpoint result, matching
fixed-U behavior before its end-of-stream callback.

## 11. Implementation strategy

1. Add `special_config.rs` under the existing protocol crate.
2. Implement minimal standard-base64 decoding for this fixed 344-byte envelope
   rather than adding a dependency.
3. Reuse `num_bigint_dig::BigUint::modpow` for raw RSA public recovery.
4. Reuse `aes::Aes256` block decryption to implement the fixed CBC operation.
5. Reuse `parse_help_config_simple` and its typed objects.
6. Export the module/API from `lib.rs`.
7. Mirror the source-neutral module/export into the desktop provider copy.
8. Author, but do not execute, protocol contracts.
9. Keep acquisition/scheduler/DC-application work explicitly blocked/pending.

## 12. Verification / test strategy

Stage A authored-only contracts:

- base64 normalization/size failures;
- AES-256-CBC known-vector decryption;
- declared-length rejection;
- expiration and empty-rule rejection;
- exact positive/negative/empty phone-prefix behavior;
- IPv4 bit-pattern, dc id, signed port and secret preservation;
- no-match returns an empty vector.

A full encrypted special-config fixture remains a Stage-B integration oracle if
one is captured from an authorized canonical source; Stage A must not perform
live acquisition solely to obtain it.

No test or validator is executed in this round.

## 13. Acceptance criteria / Definition of Done

- **AC-1:** The pure payload decoder is implemented in the existing protocol
  owner with no network/runtime/settings side effects.
- **AC-2:** Fixed-U normalization, RSA, AES-CBC, digest and declared-length
  semantics are represented exactly.
- **AC-3:** Fixed-U expiry, rule and endpoint-selection semantics are represented
  exactly.
- **AC-4:** Existing `help.configSimple` parser/types are reused rather than
  duplicated.
- **AC-5:** No new dependency or lock regeneration is required.
- **AC-6:** Source contracts are authored and remain unexecuted
  (`validation_status=not-run`, `accepted=false`).
- **AC-7:** HTTP acquisition, HTTP-Date mutation, temporary-DC request
  scheduling and runtime `DcOptions` replacement remain explicitly
  blocked/pending, not claimed as implemented.
- **AC-8:** Canonical platform and desktop provider source copies are remotely
  synchronized and evidenced through the single migration ledger line.

## 14. Release / migration / rollback

No release in Stage A. Revert the isolated protocol module/export and associated
bookkeeping commits if the fixed-U interpretation is disproven before Stage B.

## 15. Observability / evidence

Evidence is source identity, remote blob identity and authored contract identity.
There is intentionally no runtime/CI evidence in Stage A.

## 16. References / provenance

- fixed-U `Telegram/SourceFiles/mtproto/config_loader.h@13a80b1b049f28866c3db110df5c662e8e74dede`
- fixed-U `Telegram/SourceFiles/mtproto/config_loader.cpp@5d07404ba07223016c6728b986dcba10628d7194`
- fixed-U `Telegram/SourceFiles/mtproto/special_config_request.h@91901489e778089b03187682a9556e929867b94f`
- fixed-U `Telegram/SourceFiles/mtproto/special_config_request.cpp@fdb65506b0a886bc2daeccb61090bc44be7e2d88`
- fixed-U `Telegram/SourceFiles/mtproto/details/mtproto_rsa_public_key.h@86978061d7e7368d60fd2d76e46196f06dd36135`
- fixed-U `Telegram/SourceFiles/mtproto/details/mtproto_rsa_public_key.cpp@312c8b1ddb3145d5ec3180323e8d72d50c35e526`
- fixed-U `Telegram/SourceFiles/mtproto/details/mtproto_domain_resolver.h@cb1979bbbf0454135c8668ed1e5013da85d1dedc`
- fixed-U `Telegram/SourceFiles/mtproto/details/mtproto_domain_resolver.cpp@eeac8f6a8e2df67967ea7b536fb165e2f8d983e9`
- fixed-U `Telegram/SourceFiles/mtproto/mtproto_dc_options.cpp@d8499fc1df2ed7fdb88da687a04b824d243ff49a`
- fixed-U `Telegram/SourceFiles/mtproto/facade.h@39e2f732c7df32af37ebf8222e887aec7e11b256`
- prior canonical config-simple spec `docs/specs/telegram-mtproto-config-simple-wire.md`
- prior proxy consumer spec `docs/specs/telegram-proxy-non-ui-consumers-boundary.md`

No upstream C++ is copied; this specification records behavioral provenance.

## 17. Spec compliance record

Source-level compliance review completed in Stage A. Executable validation remains intentionally deferred and is not represented as pass.

| Requirement / AC | Status | Evidence / reason |
| --- | --- | --- |
| R1 | passed | Fixed-U config_loader/special_config_request/RSA/DcOptions/domain-resolver/facade blobs were reread and recorded in this spec and desktop round115 evidence. |
| R2-R6 | passed | `special_config.rs@03ea4149703942befe59132a2019bef71e92938b` implements normalization, fixed 256-byte RSA recovery, source AES-CBC layout, constant-time digest validation, declared length and existing `parse_help_config_simple` composition. |
| R7-R9 | passed | The same module implements source expiry/empty-rules, ordered positive/negative/empty prefix evaluation and typed endpoint projection without adding later network port/secret policy. |
| R10-R12 | passed | The API is pure and side-effect free, stays in existing telegram-protocol, and reuses existing aes/num-bigint-dig/sha2/subtle dependencies with no manifest/lock delta. |
| R13 | passed | Internal and public contracts are authored at `protocol_contract.rs@cd51c5db0c2d92cd713eac5ddb80f589775ead43`; none were executed. |
| R14 | passed | HTTP/DoH/Firestore acquisition, HTTP-Date mutation, temporary-DC help.getConfig scheduling and DcOptions replacement are separately tracked as mapped/pending rather than claimed implemented. |
| AC-1-AC-5 | passed | Platform implementation commit `ba7ca87b27c3fbf7cb07ea5fa01b8d5caf67d1b4`; module `03ea4149703942befe59132a2019bef71e92938b`; lib `02853449d52bc713d6db826e88875b94c2a9c70f`. |
| AC-6 | passed | Authored-only contract commit `0ac3bdc2b3bcb47871970b654bc50dc9cefbf21d`; executable validation intentionally remains `not-run`, accepted=false. |
| AC-7 | passed | Carrier/application row is `mapped/pending` in the unique desktop ledger and no network/runtime production blob was changed. |
| AC-8 | passed | Desktop mirror/bookkeeping commit `651c199fee8b3cad83e9f97b6fd92c59cd38f996`; mirrored special-config/lib/contract blobs are byte-identical and ledger/P0/source-closure/round115 evidence were synchronized. |
