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
| daemon Invocation builder completeness | P1 | **DONE end-to-end for backend producer + CLI consumer** | All 7 AXIOM fields set in 3 builders (`backend/internal/daemon_grpc/mapping.go`, `invoke_remote.go`). Backend serializes true user-signed `DelegationProof` values into `x-easynet-delegation` and backend-signed `SessionAuthority` values into `x-easynet-session-authority`, rejects ambiguous authority, and pins decode/verify in daemon gRPC and invoke_remote frame tests. EasyNet-Cli daemon admission now parses and verifies both metadata forms in `src/services/invocation_transport/admission_facade.rs`; `tests/admission_delegation_metadata.rs` proves unary, server-stream, and bidi frame-0 consumer paths. `<self>.invoke_remote` additionally verifies inner `(caller, subject, target, ability)` authority before writing `SessionDispatch`, and preserves metadata in the session dispatch frame. | Keep `InvokeRemote` receipt/control down-frames intentionally non-user-visible unless a future audit requirement needs HTTP-layer admission receipt exposure. |
| No JSON control dependency | P1 | **DONE** | `internal/cliipc` absent; backend dials `daemon.sock` via gRPC only (`servicecontext.go:154-227`); JSON path retired (comments only). | None. |
| Product state boundary | P1 | **DONE** | Backend calls daemon for all execution, stores DB projections; owns no admission/receipt semantics (`invokeAbilityLogic.go:94`, `remote_routing.go:179`). | None. |

**Backend verdict: producer-side authority proof is complete, and the matching
EasyNet-Cli daemon consumer is now complete.** Hub/backend-mediated user-subject calls
must carry `x-easynet-session-authority`; true user-delegated calls carry
`x-easynet-delegation`. Daemon admission verifies proof integrity, issuer trust,
caller/subject/audience/scope binding, and expiry before dispatch.

---

## EasyNet-Cli (device/Hub daemon) — the migration's main line

| Step | Plan item | Priority | Status | Evidence | Remainder |
|---|---|---|---|---|---|
| 1 | Unify Hub + device under easynet-daemon | P0 | **DONE (this branch)** | `run_as_hub` runs through `easynet-daemon mode=hub`, records `DaemonOnly` (`src/facade/cli/start.rs`); `ensure_hub_config` (`daemon_config.rs`); AxonBridge kept for legacy only. 3 commits on `codex/f07-hub-device-unify-2026-06-05`. | None. |
| 2 | Promote daemon Invocation transport to first-class | P1 | **DONE** | `services::invocation_transport` module tree (`src/services/invocation_transport/`); `start_daemon_invocation_transport` (`invocation_transport/boot.rs:184`); unary/stream/bidi implemented (`daemon_invocation_service.rs:982/1104/1148`) + 115 tests. The residual "sidecar" word elsewhere refers to *plugin-host* sidecars — a different concept; must not be renamed. | None. |
| 3 | daemon SDK lifecycle + client APIs in `libeasynet_cli` | P1 | **DONE (this branch)** | `easynet_cli::daemon::{DaemonStartConfig, DaemonHandle, DaemonClient, DaemonInvocation, start_daemon, stop_daemon}` (`src/daemon.rs`); `DaemonHandle::{control_endpoint, invocation_endpoint, status, stop}`; `DaemonClient::invoke(DaemonInvocation)` submits complete unary Axon Invocation over `daemon.sock`; `DaemonClient::invoke_stream(DaemonInvocation)` opens daemon `InvokeStream`; `DaemonClient::invoke_bidi(DaemonInvocation, streams)` opens daemon `InvokeBidi`; C ABI lifecycle symbols `easynet_daemon_start/stop/status/invocation_endpoint` live in `src/ffi/daemon.rs`; `facade::cli::start` now reuses the SDK lifecycle path. | None. |
| 4 | Replace ability+args FFI with complete Invocation FFI | P2 | **DONE (this branch)** | `easynet_invocation_invoke(handle, invocation_json, out_receipt_json)`, `easynet_invocation_stream_open/cancel`, and `easynet_invocation_bidi_open/send/close/cancel` exist in `src/ffi/invocation.rs` and validate the full seven-tuple before routing through `DaemonClient`. `scripts/ffi-smoke.sh` proves daemon lifecycle, non-default invocation endpoint discovery, unary receipt/result, stream callback delivery, and bidi `fs.transfer` business data plus terminal cleanup receipt through a real daemon. Legacy `easynet_ability_invoke`, `easynet_ability_subscribe`, and `easynet_subscription_cancel` are no longer exported in ABI v3. | None. |
| 5 | JSON control caller inventory | P0 | **DONE** | `docs/json-control-caller-inventory.md` (commit `1af95db`, updated in this branch). 0 remaining MUST_MIGRATE product callers; CLI plugin migrated; FFI legacy JSON product callers retired; backend confirmed absent; G1–G12 gate. | None. |
| 6 | Demote JSON control | P3 | **DONE (active CLI code)** | No production FFI/CLI product caller constructs JSON `Invoke/Subscribe/Cancel`. `src/services/control/frames.rs` now defines only boot/status `Subscribe` and `Cancel`; `src/services/control/server.rs` handles `system.watch_boot` only; daemon-internal Axon local-tool delegation is isolated in `runtime_dispatch.rs` + `runtime_dispatch_adapter.rs`. | Keep boot/status tests green and reject any future attempt to put product ability calls back on `control.sock`. |
| 7 | Remove/shrink CLI-owned Invocation semantics | P1 | **DONE (active CLI code)** | `src/runtime/invocation.rs` now names the daemon-local type `RuntimeInvocation`; it has no `canonical_bytes()` method and no `invocation_id_of()` helper. `runtime_invocation_id()` converts the record to Axon `InvocationEnvelope`, calls Axon `canonical_invocation_bytes`, then hashes Axon-owned bytes. Legacy non-null `RuntimeCausalContext` variants fail closed because they lack Axon receipt hashes/URAs. | Remaining follow-up is semantic migration of any future non-null causal callers to Axon `ReceiptRef`; current schedule/loop/kernel callers use `Null`. |

---

## Critical path (what "finish everything" actually means)

The cross-repo work is mostly **already done**. The former EasyNet-Cli critical path
for Steps 6 and 7 is complete in active code; the EasyNet-Cli proof/cleanup gates are
now covered by `scripts/check-daemon-invocation-migration.sh`:

```
InvokeRemote receipt/control audit policy  ──(only if HTTP-layer receipts become required)──
```

- **Steps 1, 2, 5** — DONE. **Axon** — protocol docs define `subject` as the
  authority/audit principal, not execution location. **Backend producer** — done;
  it emits serialized `SessionAuthority` metadata for backend sessions and
  reserves `DelegationProof` for user-signed delegation, including the
  `<self>.invoke_remote` inner metadata path.
- **Daemon authority consumer** — DONE in EasyNet-Cli; `x-easynet-session-authority`
  and `x-easynet-delegation` are parsed and verified across unary,
  server-stream, bidi frame-0 admission paths, and invoke_remote inner dispatch
  before reverse-session forwarding.
- **Step 3 and Step 4** are now complete in this branch. Unary, server-stream,
  and bidi external FFI surfaces are covered by daemon-level smoke tests; bidi
  additionally proves real `fs.transfer` business data plus terminal
  cleanup receipt over `InvokeBidi`.
- Step 7's CLI runtime invocation is now an adapter record over Axon canonical bytes, not a canonical source.
- G8 is complete in EasyNet-Cli: migrated product callers either parse complete Invocation JSON
  or use the complete `DaemonInvocation::builder(caller, callee, ability, subject)` path, and
  direct construction is mechanically rejected outside `src/daemon.rs`.
- G12 is complete for EasyNet-Cli `main` `6f463c6826dd3ff28d44db91263504d3fbc26023`: `main`
  still contains `ability_proxy.rs`, product JSON control frame variants, and CLI-owned
  `Invocation`/`CausalContext`/`canonical_bytes`/`invocation_id_of`; the current tree removes
  those active paths and passes the migration guard.

## Baseline caveat

This ledger is current-working-tree evidence on the branches named above. EasyNet-Cli has
now been re-confirmed against `main` for the daemon Invocation migration gate. Re-confirm
EasyNet backend and EasyNet-Axon against their release baselines before packaging a cross-repo
release — a sibling branch may already hold partial work (as the backend audit branch did).
