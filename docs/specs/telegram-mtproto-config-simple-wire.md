# Telegram MTProto help.configSimple exact wire validation

Status: Stage A implementation contract; executable validation deferred.

## Context and scope

This spec closes only the fixed-U typed TL wire-decoding responsibility for the adjacent bootstrap objects:

- `ipPort#d433ad73 ipv4:int port:int = IpPort`
- `ipPortSecret#37982646 ipv4:int port:int secret:bytes = IpPort`
- `accessPointRule#4679b65f phone_prefix_rules:string dc_id:int ips:vector<IpPort> = AccessPointRule`
- `help.configSimple#5a592a6c date:int expires:int rules:vector<AccessPointRule> = help.ConfigSimple`

The canonical owner is the existing shared `telegram-protocol` crate. This slice is deliberately limited to exact typed decoding and malformed/trailing validation.

It does not implement Telegram Desktop's special-config web acquisition, decryption, HTTP-date synchronization, expiry policy, phone-prefix eligibility policy, endpoint callback/application lifecycle, connection scheduling, request history, gzip handling, or updates difference decoding.

## Fixed-U provenance and verified current state

- Upstream: `telegramdesktop/tdesktop@f985723b552b30fe1c17d377d57f44e61a15a8c4`.
- `Telegram/SourceFiles/mtproto/scheme/mtproto.tl@76225965bfc794388db241f3f760db8ab79e84f8` declares the four objects above immediately after `http_wait#9299359f`.
- `Telegram/SourceFiles/mtproto/session_private.cpp@13cbc76ec723386027b5b4dcac79999b791c4614` has no `help.configSimple` receive/application branch.
- The actual fixed-U consumer is `Telegram/SourceFiles/mtproto/special_config_request.cpp@fdb65506b0a886bc2daeccb61090bc44be7e2d88` with header `special_config_request.h@91901489e778089b03187682a9556e929867b94f`: after decrypting a `help.configSimple`, Telegram Desktop checks expiry and non-empty rules, applies phone-prefix filtering, converts raw IPv4 integers to dotted-quad strings, and emits DC endpoint callbacks.
- At the live canonical checkpoint before this spec, `native/telegram-protocol/src/api.rs` already owns adjacent exact MTProto wire parsers plus `ConfigDcDirectory` parsing, but has no constants/types/parsers for these four simple-config constructors.
- `native/telegram-protocol/src/dc.rs` owns canonical DC endpoint/directory state, but no proven equivalent exists for Telegram Desktop's special-config acquisition/decryption/phone-rule/application lifecycle.

## Goal

Provide source-neutral, side-effect-free, exact typed parsers for the four fixed-U simple-config wire objects, including nested vectors and TL bytes/string decoding, while preserving all schema values exactly and failing closed on malformed or trailing input.

## Non-goals

- Do not perform web special-config discovery or decryption.
- Do not compare `date`/`expires` to wall clock time.
- Do not interpret `phone_prefix_rules`.
- Do not convert or validate `ipv4`, `port`, `dc_id`, or secret contents at the wire layer.
- Do not mutate `DcDirectory`, transport/session/auth/request state, or endpoint selection.
- Do not add a second Telegram runtime/provider/bootstrap owner.
- Do not add `wasSent/haveSent/_resendingIds/toSend/_sentContainers` semantics.
- Do not implement `gzip_packed` or complete updates difference/differenceSlice decoding/application.

## Requirements

- R1: The canonical protocol owner SHALL export `IP_PORT_CONSTRUCTOR = 0xd433ad73`, `IP_PORT_SECRET_CONSTRUCTOR = 0x37982646`, `ACCESS_POINT_RULE_CONSTRUCTOR = 0x4679b65f`, and `HELP_CONFIG_SIMPLE_CONSTRUCTOR = 0x5a592a6c`.
- R2: The canonical owner SHALL expose a typed `IpPort` sum type that distinguishes plain and secret variants while preserving raw `ipv4:i32`, `port:i32`, and secret bytes exactly.
- R3: The canonical owner SHALL expose typed `AccessPointRule` and `HelpConfigSimple` values preserving `phone_prefix_rules`, `dc_id`, nested `IpPort` values, `date`, `expires`, and rule order exactly.
- R4: `parse_ip_port`, `parse_access_point_rule`, and `parse_help_config_simple` SHALL each require the exact fixed-U constructor(s), decode fields in schema order, and reject any trailing bytes.
- R5: Nested `vector<IpPort>` and `vector<AccessPointRule>` SHALL be decoded through the existing bounded TL reader so wrong vector constructors, negative/excessive counts, truncation, invalid TL string/bytes framing, and invalid UTF-8 strings fail closed.
- R6: The wire parser SHALL NOT impose Telegram Desktop's higher-level bootstrap policy as schema validity constraints. In particular, raw signed ints and empty/non-empty semantic policy remain higher-layer concerns.
- R7: The parser SHALL be side-effect free and SHALL NOT update DC routing, transport/session/auth/request state, wall-clock state, or phone eligibility.
- R8: Telegram Desktop-equivalent special-config acquisition/decryption/expiry/phone-rule/endpoint-application lifecycle remains explicitly fail-closed/open until an existing owner can precisely represent it.
- R9: Desktop provider protocol sources SHALL be synchronized byte-for-byte only after the canonical implementation is durably committed.
- R10: Stage A SHALL only author tests/contracts; it SHALL NOT run build, typecheck, lint, schema validation, generators, unit/contract/integration/E2E tests, package, deploy, install, Actions, checks, or statuses.

## Interface and failure contract

Canonical API:

- `IpPort::Plain { ipv4, port }`
- `IpPort::Secret { ipv4, port, secret }`
- `AccessPointRule { phone_prefix_rules, dc_id, ips }`
- `HelpConfigSimple { date, expires, rules }`
- `parse_ip_port(&[u8]) -> Result<IpPort, ApiRequestError>`
- `parse_access_point_rule(&[u8]) -> Result<AccessPointRule, ApiRequestError>`
- `parse_help_config_simple(&[u8]) -> Result<HelpConfigSimple, ApiRequestError>`

Failures:

- unknown `IpPort` constructor -> dedicated protocol error;
- wrong `accessPointRule` or `help.configSimple` constructor -> existing unexpected-constructor TL error;
- malformed/truncated vector, string, bytes or field -> existing TL error;
- bytes remaining after the complete requested object -> `ApiRequestError::TrailingBytes`;
- decoded values have no routing/bootstrap side effect by themselves.

## Architecture and ownership

- Wire schema decoding owner: existing `bhrumom/fabushi-platform-core:native/telegram-protocol`.
- Existing DC state owner: `native/telegram-protocol/src/dc.rs`; unchanged by this wire-only slice.
- Desktop consumer copy: existing `source/mahayana/mahayana-rs/providers/telegram-protocol`.
- Special-config acquisition/decryption/policy/application lifecycle: not introduced here; remains fail-closed/open.
- No new account, runtime, provider, network scheduler, storage, search, UI, or product root is introduced.

## Implementation strategy

1. Add the four fixed-U constructor constants and typed wire values to `native/telegram-protocol/src/api.rs`.
2. Add internal reader-based helpers so nested objects and standalone parser entrypoints share one exact decoder.
3. Add public exact parsers with complete trailing-byte checks.
4. Re-export the constants/types/parsers from `native/telegram-protocol/src/lib.rs`.
5. Author parser contracts covering both `IpPort` variants, nested rule/config decoding, wrong constructors, truncation at every byte boundary, malformed nested constructors, and trailing bytes; do not execute them.
6. Durably commit canonical implementation.
7. Synchronize canonical protocol files byte-for-byte into the existing desktop provider.
8. Update the existing TDRP ledger/P0/source-closure/evidence with `implemented / validation_status=not-run / accepted=false`.

## Verification strategy

Stage A source-level review will verify:

- constructor IDs and field order match fixed-U `mtproto.tl`;
- both `IpPort` constructors preserve raw values and secret bytes;
- nested vectors preserve ordering;
- malformed/truncated/wrong-constructor/trailing input fails closed;
- no special-config policy or DC routing mutation is introduced;
- desktop provider api/lib blobs are byte-identical to canonical blobs after synchronization;
- authored tests remain unexecuted.

No executable validation is permitted in this stage.

## Acceptance criteria

- AC1: all four constructor constants exist in the canonical owner.
- AC2: `IpPort`, `AccessPointRule`, and `HelpConfigSimple` preserve every fixed-U field.
- AC3: standalone and nested exact parsing succeeds for both `IpPort` variants and ordered vectors.
- AC4: wrong constructors, malformed vectors/strings/bytes, truncation, and trailing bytes fail closed.
- AC5: no special-config acquisition/decryption/expiry/phone-rule/DC-application lifecycle or historical request/container state is invented.
- AC6: desktop provider api/lib blobs are byte-identical to the canonical implementation after synchronization.
- AC7: TDRP bookkeeping records the responsibility as implemented while validation remains not-run and accepted remains false.
- AC8: Stage-B executable evidence remains pending.

## References

- `telegramdesktop/tdesktop@f985723b552b30fe1c17d377d57f44e61a15a8c4`
- fixed-U `mtproto.tl@76225965bfc794388db241f3f760db8ab79e84f8`
- fixed-U `session_private.cpp@13cbc76ec723386027b5b4dcac79999b791c4614`
- fixed-U `special_config_request.cpp@fdb65506b0a886bc2daeccb61090bc44be7e2d88`
- fixed-U `special_config_request.h@91901489e778089b03187682a9556e929867b94f`
- `docs/specs/spec-first-ai-development.md`


## Source-level implementation review

Canonical implementation commits: `cde1297cacd3861764d74a111629d002e74c9d71`, fail-closed constructor dispatch correction `46db4422aa7221b1c8048f96f1c7c47cd0c01458`, and malformed-framing contract expansion `022b578b8f7823a9af737a2f88fdf4c684fa4e56`. Canonical blobs: protocol api `0c852a5e3bc41a4ac571d713ae3d4edea19b3619`, protocol lib `04e5020e7b1e7ed46b403c151f84a095a1a17e4c`. Desktop synchronization commits: `870ec15e7760718ad7ef1eb5e578dfcef8f0e46b` and final contract sync `878329a3f54895da0c44ab33e14cf8856444e128`.

- R1: satisfied by source inspection — all four fixed-U constructor constants are present with exact IDs.
- R2-R3: satisfied by source inspection — typed `IpPort`, `AccessPointRule`, and `HelpConfigSimple` preserve raw signed integer values, strings, secret bytes, and vector order without imposing bootstrap policy.
- R4-R5: satisfied by source inspection — public parsers require exact constructors, nested decoding uses the existing bounded `TlReader`, unknown `IpPort` constructors fail before variant fields are consumed, and malformed vector constructors, invalid UTF-8 strings, truncation at every byte boundary, nested wrong constructors, and trailing bytes are covered by authored contracts.
- R6-R8: satisfied by source inspection and owner review — no expiry/time comparison, phone-prefix interpretation, IP/port/DC validation, `DcDirectory` mutation, web acquisition/decryption, transport/session/auth/request mutation, or second bootstrap owner was introduced. The Telegram Desktop `SpecialConfigRequest` lifecycle remains explicitly fail-closed/open.
- R9: satisfied by remote blob readback — desktop provider api/lib blobs after `878329a3f54895da0c44ab33e14cf8856444e128` are byte-identical to canonical api/lib blobs.
- R10: satisfied for Stage A process — contracts were authored only; no build, typecheck, lint, schema validator, generator, unit/contract/integration/E2E, package, deployment, installation, GitHub Actions, checks, or statuses were run or queried.

Acceptance criteria AC1-AC7 are satisfied at source/implementation/bookkeeping level. AC8 remains intentionally **blocked/not-run** until Stage B executable validation. No runtime claim is made for special-config acquisition/decryption/expiry/phone-prefix/DC endpoint application.


## Round 65 adjacent revalidation (2026-11-01)

Task round: `7457eb1c-11a3-4626-8563-a3e495872536 / 65`.

The live checkpoint was reread before this write at `a9cca9a441ada6738bf3418f389c58293b3ec54e`. Fixed U remains `telegramdesktop/tdesktop@f985723b552b30fe1c17d377d57f44e61a15a8c4`. The fixed-U schema still places `ipPort#d433ad73`, `ipPortSecret#37982646`, `accessPointRule#4679b65f`, and `help.configSimple#5a592a6c` immediately after `http_wait#9299359f` in `mtproto.tl@76225965bfc794388db241f3f760db8ab79e84f8`. `session_private.cpp@13cbc76ec723386027b5b4dcac79999b791c4614` still has no `help.configSimple` receive/application branch. The higher lifecycle remains in `special_config_request.cpp@fdb65506b0a886bc2daeccb61090bc44be7e2d88` / `special_config_request.h@91901489e778089b03187682a9556e929867b94f`, where acquisition/decryption, HTTP-derived time, expiry/non-empty policy, phone-prefix filtering, IPv4 conversion, and DC endpoint callbacks are applied.

Current source inspection confirms the existing `telegram-protocol` owner still exports all four exact constructor IDs, typed `IpPort`, `AccessPointRule`, and `HelpConfigSimple`, plus `parse_ip_port`, `parse_access_point_rule`, and `parse_help_config_simple`. Nested vectors continue to use the bounded `TlReader`; raw signed integers, TL strings/bytes, and vector order are preserved; standalone entrypoints require strict EOF. Existing internal authored-only contracts in `api.rs` still cover both IpPort variants, nested rules/config, malformed vector/string framing, wrong top-level/nested constructors, every-byte truncation, and trailing bytes.

Disposition remains **no production delta**. Live canonical production blobs are `native/telegram-protocol/src/api.rs@ebfb3670b5cd097b9ce288782c7954f5cfe9fc43` and `native/telegram-protocol/src/lib.rs@abab39e42f100972484191f0e52e9edd9b88c6cf`; the desktop provider copies were reread byte-identically at the same blob SHAs. This round SHALL add only an external public-API regression contract that independently pins nested exact values, strict framing/EOF, and the absence of expiry/non-empty/phone/DC-application policy from the wire parser. It SHALL be mirrored to the existing desktop provider tests and SHALL NOT be executed in Stage A.

Special-config acquisition/decryption/expiry/phone-prefix/DC endpoint application, connection scheduling, arbitrary request/container history, auth-key binding/persistence, `gzip_packed`, and complete difference/differenceSlice decoding/application remain outside this wire-only responsibility and must not be synthesized here.

No GitHub Actions/status/check query or trigger, build, typecheck, lint, schema validator, generator, unit/contract/integration/E2E test, package, deployment, installation, or runtime acceptance is permitted or claimed by this revalidation.


## Round 65 final source-level compliance

Spec-first revalidation commit is `99bb755e64a1fb96889acf0b4b673f855f5e48b6` / spec blob `a80fb4dd9dd7a6855fb28162ba03716daa000b09`. The authored-only public API regression contract is canonical commit `2d78cb1aa4ad7e7edb3a31bb596b9eb79e610d32` / blob `bc5c49ee5be958a49f16d7c14617e51e47c3961b`, mirrored byte-identically into the desktop provider at commit `231337450087a652b678747b98359914c20c8058` with the same blob.

Source-level disposition remains **no production delta**. The live canonical production blobs are api `ebfb3670b5cd097b9ce288782c7954f5cfe9fc43` and lib `abab39e42f100972484191f0e52e9edd9b88c6cf`; live desktop provider api/lib readback is byte-identical. The external contract pins the public parsers independently of their internal module tests: it preserves signed field values and binary secrets through nested config decoding, accepts empty `rules` at the wire layer rather than importing Telegram Desktop's higher-level non-empty policy, and rejects every truncation boundary, wrong top-level/nested/vector constructors, and trailing bytes for the public IpPort/AccessPointRule/help.ConfigSimple entrypoints.

The higher `SpecialConfigRequest` acquisition/decryption/HTTP-time/expiry/phone-prefix/IP conversion/DC callback lifecycle remains deliberately separate and fail-closed/open; no DC directory mutation, request-history state, connection scheduling, auth-key persistence/binding, gzip runtime, or complete difference decoder/application was introduced.

The existing ledger responsibility `TDRP-R9-MTPROTO-CONFIG-SIMPLE-WIRE-001` remains `implemented / validation_status=not-run / accepted=false`. The newly authored external contract was not executed. No GitHub Actions/status/check query or trigger, build, typecheck, lint, schema validator, generator, unit/contract/integration/E2E test, package, deployment, installation, or runtime acceptance was performed.
