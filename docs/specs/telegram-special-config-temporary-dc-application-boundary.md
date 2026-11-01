# Telegram special-config temporary-DC application boundary — Specification

Status: application blocked / protocol-DC handshake prerequisite implementable
Owner candidates: existing native/telegram-network + existing telegram-runtime session lifecycle + existing telegram-protocol DcDirectory
Last updated: 2026-11-01
Related task: Fabushi migration task 7457eb1c-11a3-4626-8563-a3e495872536 round 89 / source-evidence round 120

## 1. Context and fixed source

Fixed upstream remains telegramdesktop/tdesktop@f985723b552b30fe1c17d377d57f44e61a15a8c4.

This review is limited to the carrier-independent application half of special configuration and its exact prerequisites. The direct no-proxy HTTPS/Web-PKI/800 ms carrier remains separately blocked by docs/specs/telegram-doh-special-config-http-carrier-lifecycle-boundary.md.

Fixed-U source identity:

- Telegram/SourceFiles/mtproto/config_loader.cpp@5d07404ba07223016c6728b986dcba10628d7194
- Telegram/SourceFiles/mtproto/config_loader.h@13a80b1b049f28866c3db110df5c662e8e74dede
- Telegram/SourceFiles/mtproto/mtproto_dc_options.cpp@d8499fc1df2ed7fdb88da687a04b824d243ff49a
- Telegram/SourceFiles/mtproto/facade.h@39e2f732c7df32af37ebf8222e887aec7e11b256
- Telegram/SourceFiles/mtproto/mtp_instance.h@eeb56d29cc8316bd6600c26b6f8edf3a912601eb
- Telegram/SourceFiles/mtproto/mtp_instance.cpp@d25fe3b72298fab371854f80186ee8e4777ea087
- Telegram/SourceFiles/mtproto/session_private.cpp@13cbc76ec723386027b5b4dcac79999b791c4614
- Telegram/SourceFiles/mtproto/details/mtproto_dc_key_creator.cpp@13c57454f14b814b5923ea253048fb881ad8cf03

## 2. Fixed-U behavior

### 2.1 Selected endpoint becomes a temporary shifted DC

ConfigLoader::specialToRealDcId is historically named but returns getTemporaryIdFromRealDcId. The fixed constant is Instance::Fields::kTemporaryMainDc = 1000. A selected special endpoint for real DC d is therefore routed through logical/session DC 1000+d.

sendSpecialRequest inserts exactly one transient option through DcOptions::constructAddOne using that temporary id. The option always sets tcpo_only and sets secret only when the selected endpoint carries one.

DcOptions::constructAddOne stores the temporary option in the existing DC options owner. Serialization and normal enumeration explicitly skip temporary ids, so this routing override is transient rather than durable configuration.

### 2.2 Temporary identity is not the wire DC identity

SessionPrivate::getProtocolDcId converts a temporary shifted id back to its real DC before applying the test-mode shift and media sign. DcKeyCreator writes that protocol DC id into p_q_inner_data_dc or p_q_inner_data_temp_dc.

Therefore 1000+d is a session/routing identity only. It must not appear as the MTProto protocol DC field.

### 2.3 Request, timeout, cancellation, and session teardown

sendSpecialRequest sends bare help.getConfig through the temporary shifted session. The special-attempt timer is 6000 ms.

Every new attempt begins with terminateSpecialRequest. Termination cancels the outstanding request when present and kills the temporary shifted session when present. The same teardown runs from ConfigLoader destruction.

This is a lifecycle contract, not merely a socket read timeout: request ownership and temporary shifted-session ownership are independently addressable in fixed-U.

### 2.4 Successful application is dc_options-only replacement

specialConfigLoaded rejects an empty dc_options vector. It intentionally ignores every other Config field and calls DcOptions::setFromList only.

setFromList/processFromList(overwrite=true) builds a fresh canonical endpoint map from the returned dcOption vector and replaces the existing map if it differs, publishing per-DC change notifications. Special config therefore updates endpoint routing only; main-DC config remains authoritative for the rest of Config.

## 3. Existing-owner-first audit at the reviewed heads

### 3.1 Existing telegram-protocol DcDirectory

native/telegram-protocol/src/dc.rs@a9582e4ece93590c322febbff391ba3ff7635575 already defines TEMPORARY_MAIN_DC=1000 and derive_protocol_dc_identity. It normalizes 1000+d back to d before test-mode and media-cluster wire transformation. This is the correct pure owner for the identity rule.

### 3.2 Existing telegram-network

native/telegram-network/src/lib.rs@e0dc33a728916fb7be916956f44cfb0b670486c6 already routes connect_directory through derive_protocol_dc_identity and therefore uses the real/test wire DC for obfuscated connection-start.

However establish_auth_key currently passes the caller's raw dc_id directly to AuthKeyHandshake::prepare_req_dh_params. If called with 1000+d, or in test mode, this would write a routing id rather than the fixed-U protocol DC id into p_q_inner_data_dc. That is an exact independently correctable prerequisite in the existing network owner.

The correction must derive ProtocolDcRole::Regular identity once and pass its wire_id to the auth-key inner-data builder. No dependency, manifest, lock, scheduler, persistence, or new owner is needed.

### 3.3 Existing telegram-runtime

native/telegram-runtime/src/runtime.rs@10031c5261bb0810ad2df2fca3de2e2fe4f65301 owns one Option<EstablishedSession> transport and one Option<DcDirectory> server directory per RuntimeClient. Requests are synchronously executed by taking that session, invoking, then returning it to the client.

It does not own:

- a shifted-DC session map;
- a separately addressable temporary session;
- an outstanding request handle that another timer can cancel;
- a 6000 ms total attempt deadline spanning connection/auth/RPC;
- fixed-U-style shared real-DC auth state for a temporary shifted session;
- a dc_options-only application operation with change publication distinct from replacement of the main server directory.

Creating a second Telegram runtime/session registry solely for special config would violate existing-owner-first. Treating one synchronous socket read timeout as terminateSpecialRequest would also be non-equivalent.

## 4. Decision

The complete special endpoint -> temporary shifted session -> help.getConfig -> dc_options replacement lifecycle is blocked / no-production-delta in this round.

The blocker is not the already-separated HTTP acquisition itself. Even when a SpecialConfigEndpoint is supplied as an input, the current shipping runtime lacks the session/request lifecycle needed to represent fixed-U cancellation, teardown, auth-state sharing, and endpoint-only application without inventing a parallel subsystem.

One exact prerequisite is independently implementable and required by the existing owner: auth-key handshake protocol-DC normalization. This round may correct establish_auth_key so p_q_inner_data_dc receives the derived regular wire id for normal, temporary, production, and test identities. That correction extends the already-existing TDRP-R9-MTPROTO-PROTOCOL-DC-ID-WIRE-001 responsibility; it does not mark special-config application implemented.

## 5. Required future application owner

When the existing runtime grows a source-neutral multi-session/temporary-session capability for broader MTProto needs, special-config application must extend that owner rather than create a bootstrap-only runtime.

The eventual operation must accept an already-decoded selected SpecialConfigEndpoint and:

1. map real DC d to temporary logical id 1000+d;
2. add only that temporary tcpo_only endpoint with optional secret to the existing transient directory/session routing view;
3. create or reuse the source-neutral session/auth state according to the canonical multi-session owner;
4. send bare help.getConfig on the temporary session;
5. enforce the fixed 6000 ms attempt termination boundary;
6. on valid non-empty dc_options, replace only canonical DC endpoints while preserving unrelated Config state;
7. cancel/forget the request and tear down the temporary shifted session on timeout, next attempt, owner destruction, or completion as appropriate;
8. never persist the temporary routing override.

The direct HTTP carrier remains responsible only for discovering selected endpoints and handing them to this application boundary.

## 6. Authored contract requirements

### 6.1 Implementable prerequisite contracts

Author but do not execute contracts proving the network auth-handshake DC helper maps:

- production real 2 -> protocol 2;
- production temporary 1002 -> protocol 2;
- test real 2 -> protocol 10002;
- test temporary 1002 -> protocol 10002.

Source review must confirm establish_auth_key passes this derived id to prepare_req_dh_params while retaining directory.test_mode() for server-key selection.

### 6.2 Future application contracts

Do not add executable tests against a nonexistent runtime lifecycle. When the canonical session owner exists, authored-only Stage-A contracts must cover:

- temporary id mapping 1000+d and non-persistence;
- tcpo_only and optional secret preservation;
- bare help.getConfig dispatch on the temporary session;
- 6000 ms total termination, not per-read timeout multiplication;
- request cancellation and temporary-session teardown;
- empty dc_options rejection;
- dc_options-only replacement while unrelated main Config fields remain untouched;
- exact endpoint flag/secret parsing;
- repeated endpoint attempts without durable temporary-directory leakage;
- no second DC directory, session registry, or bootstrap runtime.

## 7. Acceptance criteria for round 89 review

- AC-1: fixed-U sendSpecialRequest/specialConfigLoaded/terminate and DcOptions constructAddOne/setFromList semantics are pinned by blob SHA.
- AC-2: temporary id mapping and protocol DC normalization are pinned through facade/session/key-creator source.
- AC-3: current DcDirectory/network/runtime owners are inspected at the exact starting heads.
- AC-4: the auth-handshake raw shifted-id prerequisite is corrected only in the existing telegram-network owner, with authored unexecuted contracts and no manifest/lock delta.
- AC-5: complete special-config temporary-session application remains mapped/pending because the current runtime lacks an equivalent addressable temporary-session/request-cancel lifecycle.
- AC-6: no fake synchronous helper, second runtime, second directory, or invented persistence is added.
- AC-7: the direct HTTP carrier blocker remains separate and unchanged.
- AC-8: no CI/status/check query, build, typecheck, lint, validator, generator, dependency resolution, lock regeneration, test execution, package, deploy, installation, or runtime acceptance is performed.

## 8. Provenance

Behavioral provenance only; no upstream C++ is copied.


## 9. Round 89 source-compliance record

| Requirement / AC | Source-level status | Evidence |
| --- | --- | --- |
| AC-1 / fixed-U application semantics | passed for review | config_loader.cpp@5d07404ba07223016c6728b986dcba10628d7194 and mtproto_dc_options.cpp@d8499fc1df2ed7fdb88da687a04b824d243ff49a pin the 1000+d temporary route, 6000 ms terminate boundary, empty-dc_options rejection, endpoint-only setFromList replacement, per-DC changed publication, and non-persistence of temporary endpoints. |
| AC-2 / wire identity | passed for review | facade.h@39e2f732c7df32af37ebf8222e887aec7e11b256, session_private.cpp@13cbc76ec723386027b5b4dcac79999b791c4614 and mtproto_dc_key_creator.cpp@13c57454f14b814b5923ea253048fb881ad8cf03 prove temporary routing id is normalized before p_q_inner_data protocol DC emission. |
| AC-3 / existing owners | passed for review | DcDirectory dc.rs@a9582e4ece93590c322febbff391ba3ff7635575, network lib starting blob e0dc33a728916fb7be916956f44cfb0b670486c6, and runtime.rs@10031c5261bb0810ad2df2fca3de2e2fe4f65301 were inspected at the starting authority. |
| AC-4 / independently implementable prerequisite | implemented, validation not run | platform implementation commit 1d4ce62cb6a298ddaaec7cc29d123edf16e8e955 changes network lib to 6023c96c4ee96a7cae37dbd6f9be077cf88779cb. establish_auth_key derives the regular protocol identity and passes its wire_id into prepare_req_dh_params. Four focused contracts are authored in the same blob and were not executed. No manifest or lock changed. |
| AC-5 / complete temporary-session application | blocked / no production delta | runtime remains a single synchronous Option<EstablishedSession> owner without a shifted-session map, separately cancellable request handle, total 6000 ms attempt owner, or dc_options-only change-publication path. A bootstrap-only parallel runtime was not introduced. |
| AC-6 / no fake owner | passed | No second runtime, directory, persistence model, synchronous timeout surrogate, or HTTP facade was added. |
| AC-7 / HTTP carrier separation | passed | The direct NoProxy HTTPS/Web-PKI/800 ms carrier remains separately blocked under telegram-doh-special-config-http-carrier-lifecycle-boundary.md. |
| AC-8 / deferred validation | passed | No CI/status/check query, build, typecheck, lint, validator, generator, dependency resolution, lock regeneration, test execution, package, deploy, installation, or runtime acceptance was performed. |
| Desktop mirror | source-identical | bhrumom/fabushi-desktop commit dc4e1e654ad2627f5f29554e6db1e3a2c3e20aa6 carries the identical telegram-network lib blob 6023c96c4ee96a7cae37dbd6f9be077cf88779cb. |

The complete special-config carrier/application responsibility remains mapped/pending and accepted=false. The auth-handshake protocol-DC correction extends the existing implemented protocol-DC wire responsibility and remains validation_status=not-run.
