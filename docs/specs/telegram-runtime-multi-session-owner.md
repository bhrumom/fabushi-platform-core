# Telegram runtime canonical multi-session owner — Specification

Status: Stage-A implementation target / executable validation deferred
Owner: existing native/telegram-runtime RuntimeClient
Last updated: 2026-11-01
Related task: Fabushi migration task 7457eb1c-11a3-4626-8563-a3e495872536 round 90 / source-evidence round 121

## 1. Context and fixed source

Fixed upstream remains telegramdesktop/tdesktop@f985723b552b30fe1c17d377d57f44e61a15a8c4.

This scope is the carrier-independent general session lifecycle exposed by fixed-U `mtp_instance.cpp` / `session_private.cpp`: the shifted-session map, real-DC auth/DC sharing, session kill/removal, and the fact that config enumeration creates ordinary shifted sessions rather than a special-config-only runtime.

Pinned source:
- Telegram/SourceFiles/mtproto/mtp_instance.cpp@d25fe3b72298fab371854f80186ee8e4777ea087
- Telegram/SourceFiles/mtproto/mtp_instance.h@eeb56d29cc8316bd6600c26b6f8edf3a912601eb
- Telegram/SourceFiles/mtproto/session_private.cpp@13cbc76ec723386027b5b4dcac79999b791c4614
- Telegram/SourceFiles/mtproto/session_private.h@62b01e38dcb21832125b67dcd35fba3194a26829
- Telegram/SourceFiles/mtproto/config_loader.cpp@5d07404ba07223016c6728b986dcba10628d7194
- Telegram/SourceFiles/mtproto/mtproto_dc_options.cpp@d8499fc1df2ed7fdb88da687a04b824d243ff49a

## 2. Fixed-U behavior

1. `Instance::Private::_sessions` is a general `ShiftedDcId -> Session` map, not a special-config subsystem. `getSession` reuses an existing session or creates it through `startSession`.
2. `killSession` removes any non-main shifted session through `scheduleSessionDestroy`; teardown is generic and special-config is only one caller.
3. `getDcById` normalizes temporary routing identity 1000+d to real d for Dcenter/auth ownership, so temporary and ordinary routing identities can share the same real-DC auth state.
4. Normal config enumeration uses `configEnumDcIds` and `MTP::configDcId(dc)` over the same generic session owner. Temporary ids are excluded from canonical config enumeration/persistence.
5. Request cancellation is separate from session teardown. The complete special-config lifecycle additionally needs an independently addressable request handle and a 6000 ms total-attempt owner; those are not part of this narrower session-registry extraction.

## 3. Existing-owner-first decision

The canonical target is the existing `native/telegram-runtime::RuntimeClient`. It already owns the account-bound established transport, authorization state, resumable auth-key material, server directory, persistence, and all shipping RPC consumers.

A second runtime, special-config registry, or Telegram source-named subsystem is prohibited.

The current `Option<EstablishedSession>` is therefore replaced internally by a source-neutral logical-session owner inside the same RuntimeClient. Existing public ABI and current primary-session behavior remain unchanged.

## 4. Required owner contract

The owner SHALL:
- hold zero or more logical sessions keyed by logical/routing DC id;
- maintain one primary logical-session identity for existing callers;
- derive a canonical auth DC for each logical id using the existing protocol DC identity rule, so real 2 and temporary 1002 share auth DC 2;
- keep logical identity distinct from auth identity;
- let the current primary transport be taken/reinserted without creating a second RuntimeClient;
- support generic logical-session removal semantics without special-config policy;
- keep temporary/non-primary sessions transient unless a future separately specified persistence contract explicitly requires otherwise;
- preserve the existing persisted primary auth-key state and encrypted persistence format.

## 5. Explicit non-goals / remaining blockers

This round does NOT implement:
- direct HTTP/DoH/Firestore carrier acquisition;
- a special-config-only command or second runtime;
- asynchronous outstanding-request handles;
- the fixed 6000 ms total-attempt timer/cancellation boundary;
- dc_options-only replacement/change publication;
- generic auth export/import across independently authorized guest DCs;
- broader connection racing/reconnect scheduling.

Those remain separate responsibilities. In particular, `TDRP-R9-MTPROTO-SPECIAL-CONFIG-CARRIER-APPLICATION-001` remains mapped/pending after this owner lands.

## 6. Shipping wiring

All existing RuntimeClient RPC paths continue to call the existing `take_transport` / `put_transport` helpers, but those helpers SHALL now delegate to the canonical session owner. Therefore the current shipping primary session is a real consumer of the multi-session owner rather than dead future-only scaffolding.

## 7. Authored contracts (do not execute in Stage A)

Author source-level unit contracts proving:
- logical 2 -> auth DC 2;
- logical temporary 1002 -> auth DC 2;
- ordinary and temporary logical routes remain distinct while sharing auth DC;
- an empty owner has no primary session;
- primary identity is tracked by the same owner that stores sessions;
- no executable validation is run in Stage A.

Future contracts, only after the missing lifecycle exists, must cover secondary session creation/removal, independently cancellable request ownership, 6000 ms total-attempt teardown, transient temporary endpoint routing, and dc_options-only application.

## 8. Acceptance criteria

- AC-1: fixed-U generic shifted-session/auth-sharing/kill/config-enumeration semantics are pinned by exact blob SHA.
- AC-2: existing RuntimeClient is the sole lifecycle owner; no second runtime/registry is introduced.
- AC-3: current primary shipping transport is migrated behind the new owner with no ABI/manifest/lock delta.
- AC-4: logical temporary 1000+d can be represented independently of real auth DC d.
- AC-5: contracts are authored but not executed.
- AC-6: special-config remains pending for request cancellation/6000 ms/dc_options application and the separately blocked HTTP carrier.
- AC-7: no Actions/status/check query or executable validation is performed.


## 10. Round 90 source-compliance record

| Requirement / AC | Source-level status | Evidence |
| --- | --- | --- |
| AC-1 / fixed-U generic lifecycle | passed for source review | mtp_instance.cpp@d25fe3b72298fab371854f80186ee8e4777ea087 pins the general _sessions/getSession/startSession/killSession lifecycle; getDcById maps temporary logical ids to real Dcenter/auth identity. config_loader.cpp@5d07404ba07223016c6728b986dcba10628d7194 and mtproto_dc_options.cpp@d8499fc1df2ed7fdb88da687a04b824d243ff49a prove ordinary config enumeration is another shifted-session consumer. |
| AC-2 / unique existing owner | passed | RuntimeClient remains the sole account-bound runtime owner. No second runtime/provider/account/directory/session registry was added. |
| AC-3 / shipping primary migration | implemented, validation not run | platform implementation commit 8c57554ade7ad4bc75e3692a0e5dc57099162206 changes runtime.rs to d531db9176a09e973690b3a44da52208e09e33b5. Existing take_transport/put_transport and all current RPC paths now delegate to RuntimeClient.sessions. No public ABI, manifest, lock or storage-schema delta was made. |
| AC-4 / logical versus auth DC | implemented, validation not run | SessionRoute uses the existing derive_protocol_dc_identity(..., Regular) rule; authored contracts pin logical 2 -> auth 2 and logical 1002 -> auth 2 while keeping their logical ids distinct. |
| AC-5 / contracts authored only | passed for Stage A | Two focused internal runtime contracts are present in runtime.rs@d531db9176a09e973690b3a44da52208e09e33b5 and were not executed. |
| AC-6 / special-config remains separate | passed | TDRP-R9-MTPROTO-SPECIAL-CONFIG-CARRIER-APPLICATION-001 remains mapped/pending. Secondary endpoint session creation/auth sharing, cancellable request ownership, 6000 ms total-attempt teardown, dc_options-only application and direct HTTP carrier remain open. |
| AC-7 / deferred validation | passed | No Actions/status/check query, build, typecheck, lint, validator, generator, dependency resolution, lock regeneration, test execution, package, deploy, installation or runtime acceptance was performed. |
| Desktop mirror/bookkeeping | source-identical / recorded | desktop mirror commit 8f04c5f35cc6674b33f27edb09fe66fb1db7d6f0 carries runtime blob d531db9176a09e973690b3a44da52208e09e33b5. Desktop bookkeeping commit b8b5505c71c7e72806ded1255daaeac68163b039 adds the unique ledger row and round121 evidence with validation_status=not-run and accepted=false. |

The canonical multi-session owner is implemented only at source level. Executable verification and acceptance remain deferred to Stage B/C.
