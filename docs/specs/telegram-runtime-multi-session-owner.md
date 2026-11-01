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


## 11. Round 91 per-real-DC auth-material and secondary-session extension

This addendum narrows the next carrier-independent sub-responsibility. It supersedes only the Round 90 statements that secondary session creation/auth sharing were wholly future work; request cancellation, total-attempt timing, endpoint-only config application, and HTTP acquisition remain open.

### 11.1 Additional fixed-U source

- Telegram/SourceFiles/mtproto/details/mtproto_dcenter.h@651dcf171544745efc0b5d53afd8af4a57fa5a88
- Telegram/SourceFiles/mtproto/details/mtproto_dcenter.cpp@ef0df612db798338d166f47087bb35c1d29ff72b
- Telegram/SourceFiles/mtproto/session.h@d6f197d233fa4dcc6d2b19745e6d2787c5227911
- Telegram/SourceFiles/mtproto/session.cpp@40d7048c3057c7c5523b786048670524c8bebe4c
- Telegram/SourceFiles/mtproto/session_private.cpp@13cbc76ec723386027b5b4dcac79999b791c4614
- Telegram/SourceFiles/mtproto/mtp_instance.cpp@d25fe3b72298fab371854f80186ee8e4777ea087

Fixed-U `getDcById` maps temporary logical 1000+d to the same real Dcenter d used by ordinary logical d. SessionData/Session delegate key access to that Dcenter. Dcenter keeps auth-key state independently of any one Session, and SessionPrivate first reuses the real-DC key before initiating key creation. Therefore closing the source-neutral runtime equivalent requires auth material to be owned per real/auth DC, not duplicated per logical session.

### 11.2 Existing-owner-first target

The existing `RuntimeClient::sessions` remains the only session/auth-material lifecycle owner. Add an in-memory auth-material table keyed by `SessionRoute.auth_dc_id`. Each material record contains the existing `AuthKey`, server salt/time needed by `resume_auth_key`, and the environment bit needed to prevent production/test cross-reuse.

No new runtime, provider, account, persistence namespace, DC directory, or source-named Telegram subsystem is permitted.

### 11.3 Required behavior

1. Primary live-session insertion SHALL refresh auth material for its real/auth DC.
2. Restored `PersistedTransportState` SHALL seed the same owner lazily before a reconnect. The encrypted `transport.v1` format remains unchanged.
3. A generic logical-session take/open primitive SHALL:
   - reuse an already-owned logical session when present;
   - otherwise derive the logical route and look up auth material by real/auth DC;
   - call existing `resume_auth_key` with the requested logical DC and caller-supplied `DcDirectory` when reusable material exists;
   - otherwise call existing `establish_auth_key` for that logical DC and directory;
   - record the resulting auth material back under the real/auth DC before returning the EstablishedSession.
4. Real 2 and temporary 1002 SHALL therefore be able to hold distinct EstablishedSession instances while sharing one real-DC auth-material record.
5. A production/test environment mismatch for an existing real-DC material SHALL fail closed rather than reuse the key across environments.
6. Removing or killing a non-primary logical session SHALL remove that EstablishedSession only. The real-DC auth material remains owned by RuntimeClient for reuse, matching fixed-U Dcenter lifetime being broader than Session lifetime.
7. Secondary auth material remains memory-only. This round MUST NOT extend the storage schema or persist temporary routing overrides/non-primary sessions.
8. The primitive accepts a caller-supplied directory, so a future special-config caller can supply a one-endpoint temporary directory without coupling this owner to HTTP/DoH/Firestore acquisition.

### 11.4 Authored-only contracts

Author but do not execute contracts proving:
- persisted primary DC 2 seeds auth material addressable by logical 1002/auth DC 2;
- logical 2 and 1002 resolve to the same material slot while remaining distinct session keys;
- removing logical 1002 does not discard the shared real-DC material;
- environment mismatch is rejected;
- primary take/put continues to use the same owner and unchanged persisted transport format.

No executable network handshake is required in Stage A; the production open path itself must call the real existing `resume_auth_key` / `establish_auth_key` functions.

### 11.5 Remaining blockers after this extension

Even after the auth-material/open primitive is implemented, `TDRP-R9-MTPROTO-SPECIAL-CONFIG-CARRIER-APPLICATION-001` remains pending for:
- an independently cancellable in-flight request owner;
- the fixed 6000 ms total-attempt termination lifecycle;
- bare help.getConfig dispatch orchestration on the temporary session;
- dc_options-only replacement/change publication;
- the separately blocked direct NoProxy HTTPS/Web-PKI carrier.

No synchronous read-timeout surrogate or special-config-only public command is introduced.

### 11.6 Round 91 source-level acceptance criteria

- R91-AC-1: the additional Dcenter/Session key reuse/creation source is pinned by exact fixed-U blob SHA.
- R91-AC-2: RuntimeClient SessionOwner becomes the sole per-real-DC auth-material owner.
- R91-AC-3: the generic production open path can return an actual secondary EstablishedSession using resume_auth_key when material exists or establish_auth_key when absent.
- R91-AC-4: persisted primary auth material can seed secondary temporary logical routing without a storage-schema delta.
- R91-AC-5: logical-session teardown retains auth material and production/test reuse fails closed.
- R91-AC-6: focused contracts are authored but not run.
- R91-AC-7: HTTP carrier, request cancellation/6000 ms, help.getConfig orchestration, and dc_options-only application remain explicitly pending.
- R91-AC-8: no Actions/status/check query, build, typecheck, lint, validator, generator, dependency resolution, lock regeneration, test execution, package, deploy, install, or runtime acceptance is performed.


## 12. Round 91 source-compliance record

| Requirement / AC | Source-level status | Evidence |
| --- | --- | --- |
| R91-AC-1 / Dcenter key lifecycle | passed for source review | fixed-U Dcenter blobs 651dcf171544745efc0b5d53afd8af4a57fa5a88 / ef0df612db798338d166f47087bb35c1d29ff72b plus Session blobs d6f197d233fa4dcc6d2b19745e6d2787c5227911 / 40d7048c3057c7c5523b786048670524c8bebe4c show temporary 1000+d resolves to the same real Dcenter and Session delegates key reuse/creation to that owner. |
| R91-AC-2 / sole auth-material owner | implemented, validation not run | platform runtime blob 4983b262346b73c3dc7270bf5feac31780c28496 adds SessionOwner.auth_materials keyed by real/auth DC; no second runtime, account, provider, directory, or persistence namespace was added. |
| R91-AC-3 / actual secondary open path | implemented, validation not run | open_logical_session is the generic production path used by primary restore and supports any logical DC. It first takes an existing logical EstablishedSession, otherwise reuses per-real-DC AuthMaterial via existing resume_auth_key, and falls back to existing establish_auth_key when absent. Generic SessionOwner::put can retain a returned secondary EstablishedSession. |
| R91-AC-4 / persisted primary seeds temporary | implemented, validation not run | unchanged PersistedTransportState seeds SessionOwner lazily; logical 2 and 1002 resolve to one auth-material slot while retaining distinct logical session keys. No storage-schema/key change was made. |
| R91-AC-5 / teardown and environment fencing | implemented, validation not run | SessionOwner::remove/take do not delete auth_materials; auth material carries test_mode and mismatched production/test reuse returns an error. |
| R91-AC-6 / contracts authored only | passed for Stage A | three focused internal contracts were added for persisted 2 -> temporary 1002 sharing, temporary removal retaining real-DC material, and environment mismatch fail-closed. They were not executed. |
| R91-AC-7 / remaining special-config lifecycle | pending by design | independently cancellable request ownership, 6000 ms total-attempt termination, help.getConfig orchestration, dc_options-only replacement/change publication, and direct HTTP carrier remain open. No special-config public command or timeout surrogate was added. |
| R91-AC-8 / deferred validation | passed | No GitHub Actions/status/check query or trigger, build, typecheck, lint, validator, generator, dependency resolution, lock regeneration, unit/contract/integration/E2E execution, package, deploy, installation, or runtime acceptance was performed. |
| Desktop mirror | source-identical | bhrumom/fabushi-desktop commit 8cebdbe5634d092773cfda8b4a0b059bd552ccbb carries the same runtime blob 4983b262346b73c3dc7270bf5feac31780c28496. |

Implementation commits are 87fb77a6b7220e88f3d376b7543934ddd0644fc5 and 292f41096e554872ee6b85d43e519a9237dcb26f. The secondary-session/auth-material prerequisite is source-implemented only; executable validation remains deferred and special-config carrier/application remains pending.


## 13. Round 92 independently addressable request ownership and 6000 ms total-attempt boundary

This addendum implements the next carrier-independent prerequisite for the existing RuntimeClient / SessionOwner. It does not add endpoint acquisition, a special-config public command, or dc_options application.

### 13.1 Additional fixed-U source

- Telegram/SourceFiles/mtproto/config_loader.cpp@5d07404ba07223016c6728b986dcba10628d7194
- Telegram/SourceFiles/mtproto/config_loader.h@13a80b1b049f28866c3db110df5c662e8e74dede
- Telegram/SourceFiles/mtproto/session.cpp@40d7048c3057c7c5523b786048670524c8bebe4c

Fixed-U ConfigLoader keeps the request handle and temporary shifted-session identity as separate, independently addressable lifecycle state. sendSpecialRequest() first terminates the prior attempt, stores the returned request id and temporary logical DC, then starts a fixed 6000 ms timer. terminateSpecialRequest() cancels the stored request id and kills the same temporary shifted session; destruction performs the same teardown. Session::cancel removes the request from queued/sent ownership independently of session destruction.

The current Fabushi EstablishedSession::invoke_raw path is synchronous: RuntimeClient takes the EstablishedSession out of SessionOwner, invoke_raw blocks on transport receive, and the owner has no request handle while the call is in flight. A per-read timeout is not equivalent to the fixed-U total-attempt boundary because connect/auth/RPC and multiple reads can each consume independent timeout windows.

### 13.2 Existing-owner-first design

SessionOwner remains the sole account-bound lifecycle owner. Add a source-neutral in-memory request table keyed by an owner-local request id. Each owned request records:
- logical/routing DC id;
- a cloneable network-attempt cancellation/deadline control;
- whether completion/cancellation removes the logical session.

No second RuntimeClient, request registry, timer runtime, provider, account, persistence namespace, or special-config-only owner is permitted.

The lower telegram-network crate may expose only the minimum transport primitive required by this owner: an absolute-deadline/cancellation control that can be passed through controlled connect/auth and invoke_raw. It is not a lifecycle owner and does not choose the 6000 ms policy.

### 13.3 Required behavior

1. RuntimeClient SHALL allocate and register the request owner before opening/resuming the logical session, so the total-attempt budget covers connect, auth-key establish/resume, and the RPC.
2. The special-config-derived policy constant SHALL remain exactly 6000 ms. The generic SessionOwner API accepts a duration; only the orchestration boundary chooses 6000 ms.
3. A controlled network attempt SHALL use one absolute deadline. Every blocking connect/read/write is bounded by the remaining budget rather than resetting a full timeout for each operation.
4. A cloned cancellation control SHALL be independently addressable while synchronous invoke_raw is running. Cancellation marks the attempt cancelled and shuts down any registered connected socket so blocking transport I/O is interrupted.
5. Controlled establish_auth_key / resume_auth_key / invoke_raw SHALL use the same attempt control. Existing uncontrolled public paths retain current behavior.
6. Request termination SHALL remove the request owner and, when its teardown policy requests it, remove the logical temporary session. The real/auth-DC AuthMaterial remains owned for reuse.
7. Successful completion of a transient request SHALL also clear request ownership and tear down the transient logical session; no temporary session or request state is persisted.
8. Closing RuntimeClient SHALL drop/cancel any still-owned request controls through SessionOwner destruction semantics.
9. This round SHALL NOT dispatch help.getConfig, apply dc_options, acquire endpoints over HTTP/DoH/Firestore, add proxy behavior, or expose a public special-config command.

### 13.4 Authored-only contracts

Author but do not execute contracts proving:
- request ids are owner-local, unique, and independently addressable;
- cancelling a request flips its control to cancelled and removes only its configured logical session;
- cancelling/removing logical 1002 retains auth material for auth DC 2;
- the special-config total-attempt policy is exactly 6000 ms;
- a network attempt control reports cancellation/deadline as terminal and uses remaining-time bounds;
- no executable validation is run in Stage A.

### 13.5 Round 92 source-level acceptance criteria

- R92-AC-1: fixed-U ConfigLoader request/timer/session teardown and Session::cancel semantics are pinned by exact blob SHA.
- R92-AC-2: SessionOwner is the only request lifecycle owner and exposes independently addressable request cancellation.
- R92-AC-3: one absolute 6000 ms attempt can span controlled connect/auth/RPC rather than relying on per-read timeout reset.
- R92-AC-4: cancellation/finish tears down the transient logical session while retaining per-real-DC AuthMaterial.
- R92-AC-5: existing uncontrolled RuntimeClient/EstablishedSession behavior remains available; no second runtime/provider/persistence namespace is introduced.
- R92-AC-6: focused contracts are authored but not run.
- R92-AC-7: help.getConfig orchestration, dc_options-only application, and direct NoProxy HTTPS/Web-PKI acquisition remain pending.
- R92-AC-8: no Actions/status/check query, build, typecheck, lint, validator, generator, dependency resolution, lock regeneration, test execution, package, deploy, install, or runtime acceptance is performed.


## 14. Round 92 source-compliance record

| Requirement / AC | Source-level status | Evidence |
| --- | --- | --- |
| R92-AC-1 / fixed-U request teardown | passed for source review | config_loader.cpp@5d07404ba07223016c6728b986dcba10628d7194 pins sendSpecialRequest -> terminateSpecialRequest-before-new-attempt -> independent request id + temporary logical DC -> fixed 6000 ms timer; config_loader.h@13a80b1b049f28866c3db110df5c662e8e74dede pins the separately stored request/current-DC fields; session.cpp@40d7048c3057c7c5523b786048670524c8bebe4c pins request cancellation independent from Session kill. |
| R92-AC-2 / sole request owner | implemented, validation not run | RuntimeClient SessionOwner in runtime blob 6cfa611cd7ba49c931f689a51d213b32725ca617 owns owner-local request ids, logical DC scope, NetworkAttemptControl and teardown policy. No second runtime/request registry/provider/account/persistence namespace was added. |
| R92-AC-3 / one total-attempt deadline | implemented, validation not run | telegram-network blob f2b364a4344f07315c6581a3052a134319aa210d exposes NetworkAttemptControl with one absolute Instant deadline; controlled connect, TLS camouflage I/O, auth establish/resume and invoke_raw all use remaining deadline bounds. The connected socket is registered before TLS so cancellation can shut down blocking I/O. |
| R92-AC-4 / cancellation and temporary teardown | implemented, validation not run | SessionOwner cancel_request removes request ownership, cancels the control and removes the configured logical session while retaining auth_materials. invoke_owned_logical_request derives logical identity from request ownership and fences reinsertion on request still being owned, preventing a cancelled in-flight temporary session from being resurrected. |
| R92-AC-5 / existing behavior preserved | implemented, validation not run | Existing uncontrolled establish_auth_key, resume_auth_key and EstablishedSession::invoke_raw remain available and delegate only where necessary; current primary RuntimeClient callers are unchanged. transport.v1, public JSON command surface, manifests and lockfiles are unchanged. |
| R92-AC-6 / contracts authored only | passed for Stage A | two network contracts cover terminal cancellation/absolute deadline and three runtime contracts cover independently addressable ids, auth-material retention on cancellation and exact 6000 ms policy. They were authored but not executed. |
| R92-AC-7 / remaining special-config scope | pending by design | no bare help.getConfig temporary-attempt orchestrator is wired yet, no dc_options-only replacement/change publication exists yet, and the direct NoProxy HTTPS/Web-PKI acquisition carrier remains separately blocked. |
| R92-AC-8 / deferred validation | passed | No GitHub Actions/status/check query or trigger, build, typecheck, lint, validator, generator, dependency resolution, lock regeneration, unit/contract/integration/E2E execution, package, deploy, installation or runtime acceptance was performed. |

Spec-first commit is 89805c2b3f24fc187dac975b67e222b0ab44dee1 / spec blob d5fa42b7aa2fe517f2ac69eeaeeb0d72e0db6876. Implementation commits are dbf24c6113fbe681e9e3fdcd912e14351e1bc588, f8acfdde4020ee6d27d2a9a7c118ba759802dfd7, 3881f5f022942d3cbef9aa1caed51ebc518960f6, and 3e5b4f9686e1182b85a809bb4224b66c684de614. Executable validation remains intentionally deferred.
