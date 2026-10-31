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
