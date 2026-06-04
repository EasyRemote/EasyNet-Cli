# Cross-Repo Migration Ledger

**Purpose.** Reconcile the `commit-plan` migration table against the *verified current
state* of the three owning repos, per the plan's own Baseline Caveat (line 69: "diff the
current working tree against main before treating the diagnosis table as a migration
spec"). Each row below was checked against real code with `file:line` evidence — not
inferred from the plan.

**Baseline.** 2026-06-05. Repos audited: `EasyNet-Axon` (main), `EasyNet-Cli`
(`codex/f07-hub-device-unify-2026-06-05`), `EasyNet` backend
(`codex/backend-daemon-invocation-builder-audit-2026-06-04`).

**Ownership (EasyNet Runtime Boundary).** Axon owns the Invocation protocol primitive,
canonical bytes, receipts, admission, stream/bidi. EasyNet-Cli daemon owns product/device
policy. EasyNet backend owns the product surface and calls the daemon's Invocation
transport. A change lands in the repo that owns the *policy*, not wherever it is convenient.

---

## Status legend

- **DONE** — implemented and tested in the current tree; no work remains.
- **PARTIAL** — substantially present; a bounded remainder is listed.
- **NOT_STARTED** — the target does not yet exist.
- **N/A — not a defect** — the plan flagged it, but verification shows it is correct as-is.

---

## EasyNet-Axon (protocol)

| Plan item | Priority | Status | Evidence | Remainder |
|---|---|---|---|---|
| Seven-field proto + `from_wire_parts` helper | P0 | **N/A — not a defect** | Proto carries all 7 AXIOM params (`core/proto/axon/v1/invoke.proto:438-445`). Helper exists + tested (`sdk/rust/src/invocation/axiom.rs:204`). The dendrite-bridge "manual rebuild" (`core/runtime-rs/dendrite-bridge/src/invoke_signed_common.rs:637`) builds a `pb::Envelope` (transport type) — a *different* type from `InvocationEnvelope` — and its canonical bytes (`client-sdk/src/domain/admission.rs:201`) are byte-for-byte identical to the SDK encoder (`axiom.rs:285`), both pinned to the same PR2 worked-example anchor (`admission.rs:674/761`; `sdk/rust/tests/ability_crud_bulk.rs:351`). | None. Forcing the bridge onto `from_wire_parts` would add a redundant allocation for zero correctness gain. Intentional, test-protected mirroring across a crate boundary — not a fork. |
| SDK runtime lifecycle naming | P2 | **DONE** | `start_server` / `ServerConfig::start()` (`sdk/rust/src/server.rs:188`); no `start_daemon` / `start_easynet_daemon` leak (`lib.rs:79`). | Optional `start_axon_runtime` alias not added; current naming already unambiguous. |
| SDK public surface (no product leakage) | P1 | **DONE** | `sdk/rust/src/lib.rs:47-85` exports only protocol/contract modules; remote_desktop/voice are session contracts (plan row 40), not daemon policy. | None. |
| Federation helpers thin | P2 | **DONE** | `federation_directory.rs:267-288` — data shapes only, no routing/session/local decisions. | None. |

**Axon verdict: nothing to change.** The single P0 row was an audit false-positive resolved
by reading both encoders.

---

## EasyNet backend (product surface)

| Plan item | Priority | Status | Evidence | Remainder |
|---|---|---|---|---|
| daemon Invocation builder completeness | P1 | **PARTIAL** | All 7 AXIOM fields set in 3 builders (`backend/internal/daemon_grpc/mapping.go:229-295`, `invoke_remote.go:563`). | `x-easynet-delegation = "stub"` (`mapping.go:129`) — **blocked** on the daemon admission gate accepting a real delegation-proof format. |
| No JSON control dependency | P1 | **DONE** | `internal/cliipc` absent; backend dials `daemon.sock` via gRPC only (`servicecontext.go:154-227`); JSON path retired (comments only). | None. |
| Product state boundary | P1 | **DONE** | Backend calls daemon for all execution, stores DB projections; owns no admission/receipt semantics (`invokeAbilityLogic.go:94`, `remote_routing.go:179`). | None. |

**Backend verdict: one blocked remainder** (delegation proof) gated by daemon work, not
actionable from the backend repo alone.

---

## EasyNet-Cli (device/Hub daemon) — the migration's main line

| Step | Plan item | Priority | Status | Evidence | Remainder |
|---|---|---|---|---|---|
| 1 | Unify Hub + device under easynet-daemon | P0 | **DONE (this branch)** | `run_as_hub` runs through `easynet-daemon mode=hub`, records `DaemonOnly` (`src/facade/cli/start.rs`); `ensure_hub_config` (`daemon_config.rs`); AxonBridge kept for legacy only. 3 commits on `codex/f07-hub-device-unify-2026-06-05`. | None. |
| 2 | Promote daemon Invocation transport to first-class | P1 | **DONE** | `services::invocation_transport` façade (`src/services/invocation_transport.rs`); `start_daemon_invocation_transport` (`axon_serve/boot.rs:184`); unary/stream/bidi implemented (`daemon_invocation_service.rs:982/1104/1148`) + 115 tests. The residual "sidecar" word elsewhere refers to *plugin-host* sidecars — a different concept; must not be renamed. | None. |
| 3 | daemon SDK lifecycle + client APIs in `libeasynet_cli` | P1 | **NOT_STARTED** | No `start_daemon` / `DaemonHandle` / `DaemonClient` / `invocation_endpoint` / `invoke()` public surface exists (grep finds only a doc-comment mention). | Build the public daemon SDK. **Gates Step 4.** Plan: do not stabilize while Step 1 was unfinished — Step 1 now done, so unblocked. |
| 4 | Replace ability+args FFI with complete Invocation FFI | P2 | **NOT_STARTED** | FFI still `easynet_ability_invoke` (ability+args) at `src/ffi/ability.rs:102/155`; no `easynet_invocation_invoke` / stream / bidi. These are the Step-5 inventory's 4 MUST_MIGRATE callers. | Add complete-Invocation FFI threading `subject`/causal context. **Depends on Step 3.** |
| 5 | JSON control caller inventory | P0 | **DONE** | `docs/json-control-caller-inventory.md` (commit `1af95db`). 4 MUST_MIGRATE, backend confirmed absent, G1–G12 gate. | None. |
| 6 | Demote JSON control | P3 | **BLOCKED** | The 4 MUST_MIGRATE callers still construct `IncomingFrame::Invoke/Subscribe` (`ffi/ability.rs:155/461`, `plugin.rs:216`). | Cannot start until Steps 3+4 give those callers a daemon-Invocation replacement. Gate = inventory G9 (empty re-sweep). |
| 7 | Remove/shrink CLI-owned Invocation semantics | P1 | **NOT_STARTED** | `src/runtime/invocation.rs:104` defines a CLI-owned `Invocation` with its **own** `canonical_bytes()` (:157) + `invocation_id_of` (:200) — a second canonical source. **Live**, used by `runtime/kernel.rs:41` and `easynet-daemon.rs:51` (not dead code). | Reduce to an adapter over Axon canonical bytes, or prove equivalence + delegate. Real refactor; touches the daemon kernel. Sequence after Step 4 so callers already use Axon Invocation. |

---

## Critical path (what "finish everything" actually means)

The cross-repo work is mostly **already done**. The genuine remaining sequence is a single
chain inside EasyNet-Cli, plus one blocked backend item:

```
Step 3 (daemon SDK)  ──▶  Step 4 (complete-Invocation FFI)  ──▶  Step 6 (demote JSON control)
                                                              └─▶  Step 7 (shrink CLI Invocation)

backend delegation proof  ──(blocked on daemon admission gate format)──
```

- **Steps 1, 2, 5** — DONE. **Axon** — nothing to change. **Backend** — done except one
  blocked item.
- **Step 3** is the next actionable unit (now unblocked by Step 1). It gates Step 4, which
  gates Steps 6 and 7.
- Step 7's CLI `Invocation` is a live kernel type, so it is a real refactor, not a deletion.

## Baseline caveat

This ledger is current-working-tree evidence on the branches named above. Re-confirm against
each repo's `main` before treating any NOT_STARTED row as greenfield — a sibling branch may
already hold partial work (as the backend audit branch did).
