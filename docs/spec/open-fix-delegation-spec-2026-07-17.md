# Open Fix Delegation Spec — 2026-07-17

Status: Historical delegation snapshot. It is superseded for Canonical Runtime
Convergence V2 by `canonical-runtime-convergence-v2.md` Section 12 and the
2026-07-18 closure report.

This document was the handoff SPEC for delegating the EasyNet-Cli repair work
visible on 2026-07-17. It remains an operational record: every work packet
points to the owning tracker item, the files or repos inspected first, the
shape of the proposed implementation, and its acceptance gate.

Packets outside RF-1 through RF-9 remain independent backlog under their own
tracker. They neither expand the normative V2 scope nor prove that a closed V2
root fork is still live. Any future agent must inspect the actual checkout and
current tracker instead of treating this file as a current dirty-file list.

Authority chain:

- `docs/plans/to-be-fix.spec.md` is the tracker of record for numbered tasks.
- `docs/plans/to-be-fix.md` is the evidence log and historical audit trail.
- This SPEC is a delegation overlay. If it conflicts with the tracker, update
  both in the same commit and explain the correction.

Commit rule: any commit made for this work must use exactly
`Silan.Hu <silan.hu@u.nus.edu>` as author. Use pathspec commits. Do not include
Co-authored-by trailers.

Shared checkout rule: this repo currently contains unrelated or prior WIP.
Every agent must begin with `git status --short` and `git diff --name-only`.
Never revert or overwrite files outside the assigned packet.

---

## 0. Historical On-Disk State

At capture time on 2026-07-17, the working tree was not clean. The dirty files
were:

```text
src/cli/commands/join.rs
src/cli/commands/teach.rs
src/daemon/ability/builtins/agents/chat.rs
src/daemon/ability/builtins/governance/admin_status.rs
src/daemon/ability/builtins/governance/invocation_history.rs
src/daemon/ability/builtins/governance/meta.rs
src/daemon/ability/builtins/governance/network_health.rs
src/daemon/ability/builtins/governance/teach.rs
src/daemon/ability/builtins/real_invoke_tests.rs
src/daemon/axon_bridge/hot_agent_registrar.rs
src/daemon/execution/mission/invocation_gateway.rs
src/daemon/invocation/admission/grant_matcher.rs
src/daemon/persistence/local_agents.rs
src/eal/interpreter/dispatch.rs
docs/reviews/  (untracked)
```

Observed diff shape at capture time: the tracked code changes were
formatting/import-order and assertion wrapping only. This list is not a
description of the current checkout.

---

## 1. Delegation Packets

### Packet A — WIP Hygiene and Verification

Tracker relation: unnumbered current execution cleanup.

Goal: decide whether the current 14 tracked-file diff is purely formatting, then
either commit it as a formatting-only cleanup or leave it untouched with an
explicit review note.

Start here:

- `git diff --stat`
- `git diff --check`
- `git diff -- src/cli/commands/join.rs ... src/eal/interpreter/dispatch.rs`
- inspect `docs/reviews/` before deciding whether it belongs to this packet.

Implementation rules:

- Do not add semantic changes.
- If rustfmt produced the diff, normalize by running `cargo fmt` once.
- If any semantic change is discovered, stop and split it into a named packet
  below instead of burying it in formatting.

Acceptance:

```bash
cargo fmt --check
cargo check --lib --features axon-pb
cargo clippy --lib --features axon-pb -- -D warnings
```

Optional targeted tests, if time permits:

```bash
cargo test -q daemon::ability::builtins::governance:: --lib --features axon-pb
cargo test -q eal::interpreter:: --lib --features axon-pb
```

Done means: the diff is classified, tests are recorded, and either the
format-only pathspec is committed or the handoff note names the semantic owner.

### Packet B — Session Lifecycle State Machine

Tracker items: `T1.1`, `T1.2`, and the still-open half of `T0.4`.

Goal: make device session lifecycle explicit and then add claimant-fingerprint
conflict detection. This is the highest-value CLI-side reliability packet.

Start here:

- `docs/plans/to-be-fix.spec.md` rows `T0.4`, `T1.1`, `T1.2`
- `src/services/invocation_transport/session_initiator.rs`
- `src/services/invocation_transport/session_initiator/`
- `src/services/invocation_transport/boot/`
- existing session/open/frame0 tests under `src/services/invocation_transport`

Required design:

- Introduce a single explicit state machine:
  `Idle -> Dialing -> Preluding -> Live -> Backoff`.
- Model close causes as data, not strings:
  `Healthy`, `DisplacedSuspect`, `NoAdmissionReceipt`, `ContractSkew`,
  `Errored`.
- State transitions must be centralized in one module. Callers request
  transitions; they do not mutate scattered booleans.
- The heartbeat loop from `F-049` belongs to `Live` state ownership.
- Frame0 must carry the admission/session facts required by the tracker:
  contract version, hub session id, displaced prior marker where applicable.
- Claimant detection must use a boot nonce or equivalent stable claimant
  fingerprint stored in the presence slot.

Forbidden:

- No second lifecycle model in tests.
- No compatibility layer whose only purpose is preserving the old implicit
  architecture.
- No stringly state in public logic. String rendering is allowed only at the
  op-event or diagnostics boundary.

Acceptance:

```bash
cargo test -q services::invocation_transport:: --lib --features axon-pb
cargo test -q session --lib --features axon-pb
cargo clippy --lib --features axon-pb -- -D warnings
```

Specific proof points:

- Illegal transitions have tests.
- Reconnect after healthy close is not classified as displacement.
- Two claimants alternating within the configured window emit
  `claimant_conflict`.
- Existing session transport tests do not regress.

### Packet C — Invocation Carrier Unification

Tracker items: `T2.1`, `T2.1b`, `F-004`, `F-040`, `F-044`.

Goal: remove the remaining JSON-in-protobuf session dispatch carrier drift and
cut backend remote invocation over to canonical Invocation seven-tuple input.

Start here:

- `docs/plans/to-be-fix.spec.md` rows `T2.1`, `T2.1b`
- `docs/json-control-caller-inventory.md`
- `docs/t2.1b-backend-cutover-prep-2026-06-12.md` if present in the checkout
- `src/services/invocation_transport/`
- backend repo files named in the prep doc: `daemon_grpc`, `invoke_remote.go`,
  `remote_routing`, and contract tests.

Required design:

- Axon owns protocol schema. CLI consumes generated/SDK types; it must not hand
  copy a second wire shape.
- Keep one rolling-upgrade window only: dual-read/single-write if required,
  then delete the legacy path in the same packet or a clearly sequenced followup.
- Backend must submit the full Invocation seven-tuple to the daemon-facing
  invocation surface.
- `runtime.invoke_remote` wrapper and hand-copied structs must be retired once
  backend cutover is accepted.
- Clean stale `cliipc` comments at all three known sites named by `F-044`.

Forbidden:

- No new JSON control carrier for Invoke/Subscribe/OpenBidi.
- No fallback that silently forges `origin_caller`.
- No partial replacement that leaves SDK, backend fork, and CLI shape as three
  simultaneous truth sources.

Acceptance:

CLI:

```bash
cargo test -q services::invocation_transport:: --lib --features axon-pb
cargo check --features axon-pb
cargo check --no-default-features
```

Backend, in the EasyNet repo:

```bash
go test ./...
rg "runtime.invoke_remote|cliipc|invoke_remote.go"
```

The `rg` command may only return deliberate migration notes that name the new
owner and deletion date. Otherwise the packet is not done.

### Packet D — Axon Fork Retirement: Answer Codec and Federation

Tracker items: `T2.2c`, `T2.2d`, `F-015`.

Goal: finish retiring backend-owned copies of Axon protocol logic by moving the
answer codec and federation paths onto SDK-owned types/functions.

Start here:

- `docs/plans/to-be-fix.spec.md` rows `T2.2c`, `T2.2d`
- EasyNet backend `internal/axon/`
- EasyNet backend `federation_calls.go`
- EasyNet backend `namespace_resolve_answer.go`
- Axon SDK answer/resolve/federation APIs.

Required sequence:

1. Complete the answer codec decision and implementation first.
2. Add or extend SDK API/tests in Axon before replacing backend fork usage.
3. Replace backend consumers with SDK calls.
4. Delete obsolete fork code immediately after migration.

Forbidden:

- No half-SDK/half-fork path.
- No backend-only protocol parser.
- No compatibility wrapper if it only preserves the old fork boundary.

Acceptance:

- SDK tests cover descriptor, origin caller, answer codec, and federation
  encode/decode paths.
- Backend answer-sheet cross-domain e2e passes.
- `internal/axon` is either deleted or reduced to thin glue with no protocol
  knowledge.

### Packet E — Invocation Phase Architecture

Tracker item: `T1.6` / `F-051`.

Goal: implement the Axon runtime-rs invocation phase architecture described in
AXON-RFC-008 v3: one semantic kernel, multiple transport-specific surfaces.

Start here:

- `docs/plans/to-be-fix.spec.md` row `T1.6`
- Axon repo `core/runtime-rs/src/services/invocation/`
- AXON-RFC-008 v3 in the Axon docs.

Required design:

- Strong phase types:
  `InvokeAdmissionState`, `InvocationAuthorization`,
  `InvocationSchedulingDecision`.
- One terminal owner:
  `TerminalFinalizationService::finalize`.
- Per-geometry surfaces remain separate: unary, server stream, bidi,
  hub-forward.
- Frame/response shaping happens after finalization, not inside per-geometry
  terminal side effects.

Open decisions to settle before coding:

- Whether `RouteDecision` should be a sealed enum.
- Where SDK conformance algebra lives.
- Whether receipt projection is owned by finalize or by a projection service
  immediately downstream of finalize.

Acceptance:

- All geometries delegate terminal side effects to the one finalizer.
- No transport path writes ledger/idempotency/inflight/circuit terminal effects
  outside finalization.
- Runtime-rs invocation tests pass, and at least one regression test proves a
  terminal behavior fixed in one geometry applies to the others.

### Packet F — Workspace Split Design

Tracker item: `T4.1` / `F-010`.

Goal: design and then start the crate split for EasyNet-Cli, beginning with
persistence only after dependency ownership is proved.

Start here:

- `docs/plans/to-be-fix.spec.md` row `T4.1`
- `src/daemon/persistence/`
- `src/support/platform/`
- `src/core/`
- `Cargo.toml`

Required deliverable before code:

- A dependency graph document naming the future crates and owners for:
  config types, error types, `op_event!`, persistence stores, URA/core types,
  daemon-only runtime code.
- Explicit cycle analysis. If a cycle exists, resolve ownership in the design
  document before moving files.

Implementation rule:

- First executable slice should be persistence extraction only.
- Keep public behavior and CLI commands unchanged.
- Move-only commits must not contain semantic changes.

Acceptance:

```bash
cargo check --features axon-pb
cargo check --no-default-features
cargo test -q daemon::persistence:: --lib --features axon-pb
```

Record before/after build time and peak memory if available. If no measurable
build improvement appears, the design must still justify boundary clarity.

### Packet G — Decision-Blocked Items

Tracker items: `T0.1`, `T0.6`, `T5.8`, `T5.11`.

Goal: close decision cards before implementation agents invent policy.

Items:

- `T0.1`: RFC-001 baseline attribution needs CTO signoff on the 927 baseline
  and MCP/plugin-host attribution.
- `T0.6`: signing key access contract must answer the signing-oracle questions:
  who may request signatures, what scope is allowed, and whether each issuance
  is audited/receipted.
- `T5.8`: F-020 DEC must be signed. Until then, backend receipt rows are a
  non-authoritative read model, and UI must not call them "verified receipts".
- `T5.11`: receipt URA builder cards must be signed before adding parse-time
  builder enforcement.

Acceptance:

- DEC or RFC card is committed with the decision, rejected alternatives, and
  implementation owner.
- Any implementation packet depending on that decision links the committed DEC
  and does not restate policy from memory.

### Packet H — F-052 Frontend Lifelong Session Tests

Tracker item: `F-052`.

Goal: finish the frontend page-half tests for lifelong session routing.

Start here, in the EasyNet frontend repo:

- `AskToDoPage.tsx`
- `easynet-chat-history.ts`
- existing chat/lifelong tests around commit `21fb2e5` if available.

Required tests:

- First message sent with the lifelong sentinel binds the returned
  `lifelong_session_id`.
- Followup uses the bound UUID, never the sentinel.
- Explicit new-session action does not overwrite the lifelong pointer.

Acceptance:

```bash
npm test -- --run
npm run typecheck
```

Use the repo's actual commands if they differ. Record exact commands in the
handoff note.

---

## 2. Global Engineering Constraints

These constraints apply to every packet:

- Public behavior and public interfaces remain compatible unless the tracker
  explicitly says to remove a legacy path.
- Internal architecture should remove duplicate logic rather than add local
  patches.
- Lifecycle behavior must be modeled with explicit state machines.
- Prefer domain objects over wide parameter lists.
- Delete obsolete code immediately after migration.
- Do not keep compatibility layers unless the SPEC explicitly requires one
  rolling-upgrade window.
- Every packet must update tracker docs when it changes status.

---

## 3. Minimum Handoff Note Format

Every delegated agent should finish with this note:

```text
Packet:
Commit(s):
Files changed:
Tests run:
Tracker rows updated:
Residual risks:
Next packet unblocked:
```

If a packet is blocked, the note must name the exact missing decision, test
failure, or external repo state. "Need more context" is not sufficient.

---

## 4. Suggested Parallelization

Safe parallel tracks:

- Packet A can run immediately in this repo.
- Packet G can run in parallel because it is decision/documentation work.
- Packet H can run in the frontend repo if `AskToDoPage.tsx` is not currently
  occupied.
- Packet D can run in Axon/EasyNet backend if SDK ownership is clear.

Do not parallelize:

- Packet B and Packet C on the same CLI transport files without branch
  isolation.
- Packet C and Packet D in the backend if both edit invocation/federation
  routing.
- Packet F with any packet that is moving the same persistence or transport
  modules.

Recommended order for highest product impact:

1. Packet A — classify and stabilize current WIP.
2. Packet B — explicit session lifecycle and claimant conflict.
3. Packet C — carrier unification and backend cutover.
4. Packet E — invocation phase finalization owner.
5. Packet D — Axon fork retirement.
6. Packet F — workspace split after behavior stabilizes.
7. Packet H — frontend lifelong tests whenever the file is free.
8. Packet G — run continuously as decisions are needed.
