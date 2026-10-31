# Telegram MTProto resPQ wire contract

Status: active  
Owner: shared telegram-protocol / AuthKeyHandshake  
Last updated: 2026-11-01  
Related issue/task/PR: Fabushi migration task 7457eb1c-11a3-4626-8563-a3e495872536 round 48

## 1. Context / problem

Fixed upstream U defines `resPQ#05162463 nonce:int128 server_nonce:int128 pq:string server_public_key_fingerprints:Vector<long> = ResPQ`.
The shipping `DcKeyCreator::pqAnswered` path consumes that exact response after `req_pq_multi`, matches the request nonce,
selects a server RSA fingerprint, stores the server nonce, factorizes pq, and prepares `req_DH_params`.

The canonical Rust `AuthKeyHandshake::receive_res_pq` already delegates to `parse_res_pq`. Discovery at the live checkpoint
shows the parser already reads the constructor first, decodes the exact fixed-U fields through the bounded TL reader, validates the
Vector constructor/length, and rejects trailing bytes. The receiver already requires `AwaitingResPq`, validates the active nonce,
and rejects empty pq or an empty fingerprint vector before mutating handshake state. No product-code behavior delta is required for
this responsibility; the remaining Stage-A gap is durable specification, exhaustive authored-only contracts, and migration bookkeeping.

## 2. Goal

Close the shipping fixed-U resPQ wire responsibility in the existing `AuthKeyHandshake` owner without creating a second protocol
owner or expanding into connection scheduling, temporary/permanent key persistence, binding, renewal, or teardown.

## 3. Non-goals / out of scope

- Do not add a second Telegram handshake/runtime/provider.
- Do not change connection probing/racing, plaintext send scheduling, message-id generation, timeout, or retry.
- Do not add auth-key binding, expiry enforcement, persistence, renewal, or teardown.
- Do not add arbitrary multi-request sent/resend/container history, gzip handling, or complete updates difference decoding/application.
- Do not change the fixed-U `p_q_inner_data*`, `req_DH_params`, server-DH, or dh-gen responsibilities already tracked separately.
- Do not run build, typecheck, lint, schema validators, generators, tests, Actions, packaging, deployment, installation, or runtime acceptance in Stage A.

## 4. Requirements

- R1: Preserve `RES_PQ_CONSTRUCTOR = 0x05162463`.
- R2: `parse_res_pq` SHALL reject a complete unknown constructor immediately as `HandshakeError::UnexpectedConstructor`.
- R3: The parser SHALL decode exactly `nonce:int128`, `server_nonce:int128`, `pq:string`, and
  `server_public_key_fingerprints:Vector<long>` in that order.
- R4: The fingerprint vector SHALL require the TL Vector constructor, a non-negative bounded length, and exactly that many 64-bit elements.
- R5: The parser SHALL reject every truncation point and all trailing bytes after the exact body.
- R6: `AuthKeyHandshake::receive_res_pq` SHALL remain the live state owner, require `AwaitingResPq`, require the response nonce to
  equal the active request nonce, reject empty pq, reject an empty fingerprint vector, and mutate server_nonce/state only after validation.
- R7: Server nonce is server-provided at this phase and SHALL be captured exactly, not compared to pre-existing state that does not yet exist.
- R8: Fingerprint matching/trust selection and pq factorization remain the next handshake preparation step; this slice does not move them
  into a connection/runtime owner.
- R9: Exact positive/negative contracts SHALL be authored in the existing canonical protocol contract harness and mirrored into the
  existing desktop provider contract surface. They SHALL NOT be executed during Stage A.

## 5. Verified current state

At platform-core checkpoint `3a08f54f8a49c8a8af7a23b252938d1ea4607954`:
- `native/telegram-protocol/src/handshake.rs@87616f4b96a1775455da42b0bbed77711fb3c9f6` already implements R1-R8.
- `native/telegram-protocol/src/tl.rs@89256ab35184b6dc06a095ab34abb9f44e95a264` validates Vector constructor,
  negative length and the configured vector bound.
- Existing contracts cover a valid two-fingerprint response and active-nonce mismatch, but not the full truncation/vector/trailing/empty-body matrix.

## 6. Target state

Production behavior remains in the same canonical owner. The repository additionally contains exhaustive authored-only contracts covering
the exact fixed-U body, constructor discrimination, all truncation cuts, vector constructor/length failures, trailing bytes, empty pq,
empty fingerprints and state/nonce rejection. Migration bookkeeping records the responsibility as implemented but not run/accepted.

## 7. Architecture and ownership boundaries

- `native/telegram-protocol/src/handshake.rs`: canonical parser and handshake state owner; no production change required.
- `native/telegram-protocol/src/tl.rs`: existing bounded TL primitive owner.
- `native/telegram-protocol/tests/protocol_contract.rs`: Stage-A authored-only contracts.
- Desktop tracked telegram-protocol contract surface: mirrored consumer contract only; no copied handshake core.
- telegram-network/runtime/storage: unchanged.

## 8. Data flow

`authenticated plaintext body -> parse_res_pq -> ResPq -> receive_res_pq active-nonce/basic-shape validation -> ResPqReceived -> existing req_DH_params preparation`.

## 9. Fixed provenance

- Upstream U: `telegramdesktop/tdesktop@f985723b552b30fe1c17d377d57f44e61a15a8c4`.
- Schema: `Telegram/SourceFiles/mtproto/scheme/mtproto.tl@76225965bfc794388db241f3f760db8ab79e84f8`.
- Shipping anchor: `Telegram/SourceFiles/mtproto/details/mtproto_dc_key_creator.cpp@13c57454f14b814b5923ea253048fb881ad8cf03`.
- Stage A may advance only to `implemented / validation_status=not-run / accepted=false`.

## 10. Failure cases to contract

- Unknown four-byte constructor.
- Truncation at every prefix/body cut.
- Wrong Vector constructor.
- Negative Vector length.
- Vector length above the configured bound.
- Fingerprint vector shorter than its declared count.
- Trailing bytes.
- Active nonce mismatch.
- Empty pq.
- Empty fingerprint vector.
- Calling the receiver outside `AwaitingResPq`.

## 11. Implementation strategy

1. Commit this spec before contract changes.
2. Preserve existing production implementation because source review already shows the fixed-U wire semantics are exact.
3. Author the missing contract matrix in the existing canonical harness.
4. Mirror the contract file into the desktop tracked provider surface.
5. Update the existing parity ledger, P0 task files, source-closure and one round evidence file.
6. Perform source-level compliance readback only; execute nothing in Stage A.

## 12. Verification strategy

Stage A: author and manually inspect contracts; do not execute them.
Stage B: after full migration implementation closure, run the real build/unit/contract/integration/package campaign on the frozen candidate.

## 13. Acceptance criteria

- AC-1: Canonical parser/receiver remain the only state/wire owner and preserve R1-R8.
- AC-2: Authored-only contracts cover every failure case listed above plus a valid exact response.
- AC-3: Desktop mirrors the contract surface without gaining a second handshake implementation.
- AC-4: Bookkeeping records exact source and target SHAs with `validation_status=not-run`, `accepted=false`.
- AC-5: No Stage-A executable validation or CI is triggered.

## 14. Release / rollback / observability

Checkpoint branches only. Rollback is the previous checkpoint ref. Evidence is limited to Git commits, blob SHAs and remote source readback.

## 15. References

- Fixed-U `mtproto.tl` and `details/mtproto_dc_key_creator.cpp` identities above.
- Existing `AuthKeyHandshake::receive_res_pq`, `parse_res_pq`, and bounded `TlReader`.
