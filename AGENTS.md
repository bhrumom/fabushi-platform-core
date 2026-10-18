# Fabushi Platform Core — Agent Instructions

These instructions apply repository-wide to AI-assisted development in `bhrumom/fabushi-platform-core`.

## CRITICAL: Repository ownership

This repository is the canonical source for **shared Mahayana/Rust runtime, native shared capabilities, and versioned cross-platform contracts**.

- Verify the current GitHub repository before product-affecting work.
- Do not implement another Fabushi platform's product code here. Switch to that platform's canonical repository first.
- `bhrumom/fabushi` is the legacy migration/source-history repository, not the canonical implementation repository for this scope.
- Cross-platform shared runtime/contracts belong in `bhrumom/fabushi-platform-core`; platform-specific consumers remain in their own repositories.

## CRITICAL: Spec-first development — No Spec, No Code

Before changing application/runtime code, tests, schemas, contracts, dependencies, build/release configuration, migrations, security controls, or other behavior-affecting files:

1. Read this `AGENTS.md`.
2. Find and read the applicable durable Spec/project/source-of-truth documents.
3. Check `docs/specs/` for a task/feature Spec.
4. Validate the Spec against the latest explicit user requirement and current repository/GitHub facts.
5. If no usable Spec exists, or it is stale/unclear/contradictory, create or repair the Spec **before implementation** using `docs/specs/SPEC_TEMPLATE.md`.

Read-only investigation needed to understand the system or write the Spec is allowed first. Product-affecting implementation is not.

A test file named `*.spec.*` is not automatically the durable product/development Spec required by this rule.

## CRITICAL: Standard end-to-end development lifecycle

All AI-assisted development must follow this lifecycle unless a newer explicit user instruction or an applicable repository/project specification defines a stricter or more task-specific requirement:

**Discover → Confirm Goal → Spec → Current-State Verification → Architecture → Task Decomposition → Test Design → Implement → Layered Verification → Failure/Recovery Verification → Exact-HEAD CI → Packaged Acceptance → Independent Acceptance → Spec Compliance Review → Protected Integration → Canonical-Main Verification → Release → Post-Release Smoke Test → Evidence Archive → COMPLETE**

The lifecycle is fail-closed: a later stage is not successful when a required earlier gate is incomplete, failed, stale, or unsupported by evidence. A stage may be marked not applicable only with a recorded reason.

### Stage 0 — Discover
Read repository instructions, locate the owning source of truth/Spec/tasks/decisions/contracts/evidence, and inspect the current code and live GitHub state. Do not treat chat memory, an old branch, or an old work-session summary as current state.

### Stage 1 — Confirm Goal
Record the current observable problem/requested change, target externally observable outcome, scope, non-goals, and affected architecture/protocol/schema/UI/security/performance/migration/package/release surfaces.

### Stage 2 — Spec
Read the durable Spec completely. Create or repair it before implementation if missing, stale, incomplete, or contradictory. Define requirements, edge cases, verification, acceptance criteria, and Definition of Done before coding. Use stable IDs for non-trivial work. The Spec records durable truth, not minute-by-minute development history.

### Stage 3 — Current-State Verification
Verify the canonical branch/SHA, active branch/PR head SHA, actual code/module ownership, dependency/protocol versions, conflicting work, relevant CI, current version, and published artifacts when applicable. Do not implement against an unverified repository shape.

### Stage 4 — Architecture
Derive the design from the Spec. Define ownership and runtime/process boundaries, dependency direction, interfaces, data/control flow, cancellation, timeout, retry, reconnect/resync, crash settlement, migration, and observability as applicable. Record important intentional decisions durably.

### Stage 5 — Task Decomposition
Split non-trivial work into small traceable tasks with independently verifiable outcomes and map them to requirement/acceptance IDs.

### Stage 6 — Test Design
Before implementation, define how each requirement will be proven using applicable layers:

**Static/Architecture → Unit → Contract → Integration → E2E → Regression → Failure/Recovery → Packaged App → Release/Update Acceptance**

Behavioral test scope follows the latest explicit user instruction and applicable Spec. Do not invent a behavioral release gate the user explicitly waived, and do not skip one the user or Spec explicitly requires.

### Stage 7 — Implement
Implement against the durable Spec and verified source. Stay in scope, preserve architecture boundaries, do not weaken requirements merely to make checks pass, and update the Spec/decision record before or together with intentional behavior/design changes.

### Stage 8 — Layered Verification
Run required verification from the cheapest/narrowest useful layer toward broader layers, diagnose failures at the lowest explanatory layer, and verify affected regressions. A source-level pass is not an application/package pass. Waived behavioral layers must be recorded as not run/waived, not passed.

### Stage 9 — Failure and Recovery Verification
When applicable, verify network interruption/recovery, timeout/cancel, process crash/restart, app restart, reconnect/resync, stale/duplicate events, partial failure, authentication/OAuth failure, MCP/tool failure, and migration interruption/rollback.

### Stage 10 — Exact-HEAD CI
Bind authoritative CI evidence to the exact source revision being accepted. Record the PR/branch head SHA and workflow run ID/URL. Evidence from an earlier SHA is stale after the head changes. When repository policy requires GitHub Actions or another designated runner, local results are supplementary only.

### Stage 11 — Packaged Acceptance
When required by the Spec or latest user instruction, verify the actual installable/deployable artifact: build/package, embedded binaries/resources, signing, notarization/stapling, updater metadata, checksums, install/launch, and required critical flows as applicable. A source build alone does not prove the distributed package.

### Stage 12 — Independent Acceptance
For non-trivial, release-bound, architecture-sensitive, or high-risk work, use an acceptance pass independent from implementation reasoning and compare:

**Spec ↔ Final Diff/Code ↔ Exact-Source CI ↔ Required Packaged Behavior ↔ Evidence**

Do not treat the implementation session's completion claim as proof.

### Stage 13 — Spec Compliance Review
Map every applicable requirement/acceptance criterion to `passed` with evidence, `blocked` with reason, or `not-applicable` with reason. Any intentional divergence must already be reflected in the durable Spec/decision record.

### Stage 14 — Protected Integration
Use: development branch → PR → exact-HEAD required CI → required packaged acceptance → review/independent acceptance → Spec compliance → protected merge → canonical main. Do not merge merely because code was written, pushed, or partially tested.

### Stage 15 — Canonical-Main Verification
Treat the canonical-main merge SHA as the new integrated source identity. Do not assume PR-head evidence proves the merged revision. Verify post-merge build/release workflows bind to the exact canonical-main SHA when applicable.

### Stage 16 — Release
Release only from the accepted canonical source revision and satisfy applicable version, build/package, signing/notarization, metadata, checksum, artifact/store publication, and rollback-readiness gates. A PR merge is not a release and a candidate artifact is not a completed release.

### Stage 17 — Post-Release Smoke Test
When required by the latest explicit user instruction or applicable Spec, obtain the artifact from the real distribution channel and exercise the required critical path. If behavioral testing was explicitly waived, record the waiver truthfully rather than fabricating a pass.

### Stage 18 — Evidence Archive
Preserve applicable final canonical SHA, PR/merge reference, workflow run IDs/URLs, release version/tag, artifact IDs/checksums, test reports, screenshots/video, logs/traces, migration/rollback results, and known limitations/deferred work. Detailed action history belongs in commits, PRs, task/work logs, and CI; the Spec remains focused on durable truth and final compliance/evidence references.

### Stage 19 — COMPLETE Gate
A task may be marked `COMPLETE` only when all applicable requirements and required delivery gates are satisfied and evidence-backed.

> **No Spec, No Code.**
>
> **No Evidence, No Complete.**
>
> **No required exact-source verification, No Acceptance.**
>
> **No required canonical-main packaged/release verification, No Release Complete.**

Partial implementation, a green unit suite, a successful PR build, a merged PR, or an uploaded candidate artifact is not by itself sufficient to claim end-to-end completion.


## Fail-closed rules

Do not:
- start product-affecting implementation without reading a usable Spec;
- use chat memory as the only persistent requirement source for substantive work;
- silently expand/reduce scope;
- weaken acceptance criteria to make checks pass;
- leave the Spec stale after intentional design/behavior changes;
- claim completion without requirement-to-evidence review.

Canonical Spec-first policy: `docs/specs/spec-first-ai-development.md`.
