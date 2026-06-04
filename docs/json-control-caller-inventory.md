# JSON Control Caller Inventory

**Document status:** P0 GATE for commit-plan Step 6 (JSON-control demotion to boot/status/lifecycle only).
**Baseline:** current working tree, branch `codex/f07-json-control-caller-inventory-2026-06-04`.
**Method:** static sweep of all `IncomingFrame` / `OutgoingFrame` construction sites across six caller categories, reconciled against an independent completeness/classification audit (zero missing, zero misclassified, backend absence confirmed).

---

## 1. Purpose & Scope

### What this document proves

Commit-plan Step 6 demotes the JSON control plane (the length-delimited frame protocol served over `control.sock` / the Windows named pipe by `src/services/control/server.rs`) so that it carries **only** boot, status, and lifecycle traffic. That demotion is blocked until we can prove that **every product-ability caller** that currently constructs an `IncomingFrame::{Invoke, Subscribe, OpenBidi, SendBidi, CloseBidi}` has a documented replacement plan on the daemon **Invocation** primitive (the Axon gRPC `InvocationClient` surface served over `daemon.sock`). This inventory is that proof.

### What a "caller" is

A **caller** is a code site that *constructs an inbound `IncomingFrame`* to drive the control plane — i.e. the client/initiator side of the JSON wire. This is distinct from:

- **Daemon-internal handlers** (`src/services/control/server.rs`, `src/services/control/ability_proxy.rs`) that *receive* `IncomingFrame` envelopes off the wire and *emit* `OutgoingFrame` responses. These are the reception/dispatch/forwarder machinery of the control plane itself, not callers, and are out of scope for migration.
- **Tests** that construct frames to pin wire-format stability.

The migration question — "does this caller need to move to the Invocation primitive?" — applies only to true product-ability callers.

### The Invocation primitive vs. JSON control

In the EasyNet ontology, the **Invocation** is the only runtime-addressable execution primitive. The daemon exposes it as an Axon gRPC service (`easynet_axon::pb::axon::v1::invocation_client::InvocationClient`, instantiated in `src/support/local_daemon_grpc.rs:417`) over `daemon.sock`. The JSON control plane is a *separate* transport over `control.sock`; product-ability calls that ride JSON control today must be re-homed onto the Invocation primitive so they participate in the canonical execution and **receipt chain**.

### The six categories

| Category | Meaning |
|---|---|
| `language_binding` | FFI entry points (`src/ffi/*`) that drive frame construction from foreign-language bindings. |
| `backend` | Go backend / `cliipc` JSON callers. **Confirmed absent in this repo** (see §2). |
| `cli_command` | `easynet <cmd>` subcommands that construct frames. |
| `boot_status` | Boot-progress / status / diagnostics flows (e.g. `system.watch_boot`) — the traffic JSON control is *retained* for. |
| `daemon_internal` | Receiver / dispatcher / forwarder handlers inside the daemon. Out of scope for migration. |
| `test` | Frame construction in test modules pinning wire-format compatibility. |

### The receipt-chain completeness rule (re: `subject`)

The `subject` field on an `IncomingFrame` carries the **causal context** — the URA identifying who/what the call is on behalf of — that the daemon threads into the `EnvelopeContext` and, ultimately, into receipt-chain envelope construction. A caller that passes `subject: None` produces an Invocation with **no causal anchor**, which breaks receipt-chain provenance.

**Completeness rule:** every product-ability caller migrated to the Invocation primitive MUST supply a real `subject` (causal context) rather than `None`. The receipt-chain gap is the load-bearing reason the FFI callers (which today all pass `subject: None`) are `MUST_MIGRATE` and not merely cosmetic ports. This is the gap flagged as PR-INVOCATION-EXEC-UNITY / AXIOM §2.

---

## 2. Executive Summary

### Caller counts by category

| Category | Caller sites | Product-ability callers | `MUST_MIGRATE` | Migration-relevant |
|---|---:|---:|---:|---|
| `language_binding` | 4 | 3 (+1 test harness) | 3 | yes |
| `backend` | 0 | 0 | 0 | **CONFIRM_ABSENT** |
| `cli_command` | 2 | 1 | 1 | partial |
| `boot_status` | 6 | 0 | 0 | retained-as-control |
| `daemon_internal` | 26 | 0 | 0 | no (handler side) |
| `test` | 40 | 0 | 0 | no (compat pins) |

### MUST_MIGRATE total: **4**

| # | Caller | Frame | Why it blocks demotion |
|---|---|---|---|
| 1 | `src/ffi/ability.rs:155` | `Invoke` | FFI RPC-style ability call; `subject: None` (`:159`) — receipt-chain gap. |
| 2 | `src/ffi/ability.rs:461` | `Subscribe` | FFI streaming ability call; `subject: None` (`:465`) — receipt-chain gap. |
| 3 | `src/ffi/ability.rs:541` | `Cancel` | FFI subscription cancel emitted from the streaming reader task. |
| 4 | `src/facade/cli/groups/plugin.rs:216` | `Invoke` | `easynet plugin` invokes the `device.plugin.reload` / `device.plugin.status` product abilities; `subject: None` (`:219`). |

One additional FFI site (`src/ffi/client.rs:327`) is `UPDATE_AFTER_MIGRATION` — a test harness that round-trips `Invoke` against a real daemon and must be re-pointed at the Invocation path post-migration, not deleted.

### Backend absence — confirmed

The `backend` category is **definitively empty** in EasyNet-Cli. Verified against the working tree:

- `find . -name '*.go'` → **zero results** (Go backend lives in the separate EasyNet repo).
- `find . -type d -name backend` → **zero results**; no `cliipc` implementation.
- CLI ability invocation routes through `InvocationClient` over `daemon.sock` (`src/support/local_daemon_grpc.rs:417`, `:346` `invoke_local_daemon_ability_with_subject`), i.e. the Axon Invocation gRPC surface — **not** JSON control.

This corroborates commit-plan line 59 (backend uses daemon gRPC over `daemon.sock`). No migration work exists in this category for this repo.

### Demotion-readiness verdict

**NOT YET READY.** Four `MUST_MIGRATE` product-ability callers (3 FFI + 1 CLI plugin) still construct product-ability `Invoke`/`Subscribe`/`Cancel` frames against JSON control. JSON-control `Invoke`/`Subscribe`/`OpenBidi` cannot be removed until all four are re-homed onto the Invocation primitive **with real `subject` causal context**, and the `ffi/client.rs:327` harness is updated. The `boot_status`, `daemon_internal`, and `test` populations impose no blockers. See §4 sequencing and §5 gate.

---

## 3. Per-Category Caller Tables

### 3.1 `language_binding` (FFI)

| Caller (file:line) | Frame(s) | Subject | Description | Verdict |
|---|---|---|---|---|
| `src/ffi/ability.rs:155` | Invoke | None (`:159`) | `easynet_ability_invoke` — RPC-style ability call; FFI entry point for language bindings. | **MUST_MIGRATE** |
| `src/ffi/ability.rs:461` | Subscribe | None (`:465`) | `easynet_ability_subscribe` (`run_subscription`) — streaming ability call. | **MUST_MIGRATE** |
| `src/ffi/ability.rs:541` | Cancel | N/A | `run_subscription_loop` — Cancel emitted when the cancellation token fires from the background reader task (`easynet_subscription_cancel` triggers this indirectly). | **MUST_MIGRATE** |
| `src/ffi/client.rs:327` | Invoke | None | FFI client test harness round-tripping Invoke against a real daemon socket; exercises the production FFI path. | UPDATE_AFTER_MIGRATION |

**Notes.** All three public FFI entry points (`easynet_ability_invoke`, `easynet_ability_subscribe`, `easynet_subscription_cancel`) are product-ability callers. **Critical:** the Invoke and Subscribe sites both pass `subject: None` (confirmed at `:159` and `:465`) — the receipt-chain gap per §1. FFI must thread `subject`/causal context from the foreign-language binding into the Invocation envelope (AXIOM §2 / EnvelopeContext path) when migrated.

### 3.2 `backend`

| Caller (file:line) | Frame(s) | Subject | Description | Verdict |
|---|---|---|---|---|
| (none) | Invoke / Subscribe / OpenBidi / SendBidi / CloseBidi | N/A | No Go backend or `cliipc` JSON caller exists in this repo. | **CONFIRM_ABSENT** |

**Evidence.** Zero `.go` files; zero `backend/` directories; CLI ability invocation routes through Axon `InvocationClient` over `daemon.sock` (`src/support/local_daemon_grpc.rs:417`, `:346`). The Go backend is housed in the separate EasyNet repository and never touches JSON control over `control.sock`.

### 3.3 `cli_command`

| Caller (file:line) | Frame(s) | Subject | Description | Verdict |
|---|---|---|---|---|
| `src/facade/cli/groups/plugin.rs:216` | Invoke | None (`:219`) | `easynet plugin` invokes the `device.plugin.reload` / `device.plugin.status` product abilities (`plugin_lifecycle_ability::RELOAD_ABILITY` / `STATUS_ABILITY`) to notify the daemon of plugin package changes. | **MUST_MIGRATE** |
| `src/facade/cli/start_boot_watcher.rs:155` | Subscribe | None (`:159`) | `easynet start` boot watcher subscribes to `system.watch_boot` (`WATCH_BOOT_ABILITY`, `src/services/control/server.rs:65`) to stream daemon boot events to the CLI UI. | UPDATE_AFTER_MIGRATION |

**Notes.** `plugin.rs:216` calls genuine product abilities (`device.plugin.*`) and must move to the Invocation primitive per Step 5. `start_boot_watcher.rs:155` subscribes to `system.watch_boot`, which is **boot/status control-plane traffic** — it is *retained* on JSON control (or moves to a dedicated boot/status endpoint per Step 6), not forced through product Invocation; it is listed here only because it physically constructs a frame from a CLI command. No `OpenBidi`/`SendBidi`/`CloseBidi` frames are constructed anywhere under `src/facade/cli/` — bidi construction appears only in daemon handlers and tests.

### 3.4 `boot_status`

| Caller (file:line) | Frame(s) | Subject | Description | Verdict |
|---|---|---|---|---|
| `src/facade/cli/start_boot_watcher.rs:155` | Subscribe | None | CLI subscribes to `system.watch_boot`; fresh connection, `subscription_id = cli-start-watch-boot`. | KEEP_AS_CONTROL |
| `src/bin/easynet-daemon.rs:100-394` | — | N/A | Daemon main creates `BootBus` and emits boot progress (`emit_started`/`emit_ok`/`emit_failed`/`emit_skipped`/`emit_ready`) across all bootstrap stages through the Ready terminal. | KEEP_DAEMON_INTERNAL |
| `src/services/control/server.rs:336-343` | Subscribe | N/A | Daemon handler detects `Subscribe` to `system.watch_boot` and spawns the boot-forwarder task. | KEEP_DAEMON_INTERNAL |
| `src/services/control/server.rs:388-461` | Frame, Terminal, Cancel | N/A | Boot-forwarder task: serializes `BootBus` events to `OutgoingFrame::Frame`, emits `Terminal(reason=done)` on `Ready`/`Failed`, honors client `Cancel`. | KEEP_DAEMON_INTERNAL |
| `src/services/control/server.rs:596-648` | Subscribe | None | Test `watch_boot_subscription_receives_ready_terminal`. | KEEP_DAEMON_INTERNAL |
| `src/services/control/boot_events.rs:274-362` | — | N/A | `BootBus` unit tests (broadcast / history replay / lag). | KEEP_DAEMON_INTERNAL |

**Notes.** Zero product-ability callers. The only CLI caller (`start_boot_watcher.rs:155`) is the legitimate boot/status flow that JSON control is being *retained* for. Daemon-side `BootBus` emission is intra-daemon bootstrap coordination, not a caller.

### 3.5 `daemon_internal` (handler side — out of scope for migration)

All 26 sites RECEIVE inbound frames and CONSTRUCT outbound responses; none construct inbound product-ability frames.

| Caller (file:line) | Frame(s) | Subject | Description | Verdict |
|---|---|---|---|---|
| `src/services/control/server.rs:283` | Error | N/A | Protocol error: malformed `IncomingFrame` on wire decode. | KEEP_DAEMON_INTERNAL |
| `src/services/control/server.rs:357` | Error | Some | Booting error for Invoke (request_id passed through). | KEEP_DAEMON_INTERNAL |
| `src/services/control/server.rs:365` | Error | Some | Booting error for Subscribe (subscription_id). | KEEP_DAEMON_INTERNAL |
| `src/services/control/server.rs:371` | Error | Some | Booting error for Cancel (subscription_id). | KEEP_DAEMON_INTERNAL |
| `src/services/control/server.rs:379` | ErrorBidi | Some | Booting error for Bidi (session_id). | KEEP_DAEMON_INTERNAL |
| `src/services/control/server.rs:415` | Frame | N/A | Boot forwarder: Frame per BootBus event. | KEEP_DAEMON_INTERNAL |
| `src/services/control/server.rs:434` | Frame | N/A | Boot forwarder: Frame for broadcast-lag event. | KEEP_DAEMON_INTERNAL |
| `src/services/control/server.rs:455` | Terminal | Some | Boot forwarder: Terminal on stream complete/cancel. | KEEP_DAEMON_INTERNAL |
| `src/services/control/ability_proxy.rs:363` | Error | Some | Cancel handler: subscription_id not in active registry. | KEEP_DAEMON_INTERNAL |
| `src/services/control/ability_proxy.rs:404` | ErrorBidi | Some | SendBidi: payload not JSON-encodable. | KEEP_DAEMON_INTERNAL |
| `src/services/control/ability_proxy.rs:429` | ErrorBidi | Some | SendBidi: handler input channel closed before delivery. | KEEP_DAEMON_INTERNAL |
| `src/services/control/ability_proxy.rs:439` | ErrorBidi | Some | SendBidi: session_id not in per-connection registry. | KEEP_DAEMON_INTERNAL |
| `src/services/control/ability_proxy.rs:491` | Error | Some | Subscribe: ability resolver failure. | KEEP_DAEMON_INTERNAL |
| `src/services/control/ability_proxy.rs:511` | Error | Some | Subscribe: dispatcher failure (NOT_FOUND / ABILITY_FAILED). | KEEP_DAEMON_INTERNAL |
| `src/services/control/ability_proxy.rs:559` | ErrorBidi | Some | OpenBidi: session_id already in use on this connection. | KEEP_DAEMON_INTERNAL |
| `src/services/control/ability_proxy.rs:579` | ErrorBidi | Some | OpenBidi: ability resolver failure. | KEEP_DAEMON_INTERNAL |
| `src/services/control/ability_proxy.rs:598` | ErrorBidi | Some | OpenBidi: dispatcher failure (NOT_FOUND / ABILITY_FAILED). | KEEP_DAEMON_INTERNAL |
| `src/services/control/ability_proxy.rs:788` | Error | Some | Invoke: ability resolver failure. | KEEP_DAEMON_INTERNAL |
| `src/services/control/ability_proxy.rs:820` | Result | Some | Invoke: successful Result with optional `receipt_header`. | KEEP_DAEMON_INTERNAL |
| `src/services/control/ability_proxy.rs:833` | Error | Some | Invoke: dispatcher failure (NOT_FOUND / ABILITY_FAILED). | KEEP_DAEMON_INTERNAL |
| `src/services/control/ability_proxy.rs:943` | Error | Some | Subscribe forwarder: stream payload not JSON-decodable. | KEEP_DAEMON_INTERNAL |
| `src/services/control/ability_proxy.rs:954` | Frame | Some | Subscribe forwarder: Frame per stream value. | KEEP_DAEMON_INTERNAL |
| `src/services/control/ability_proxy.rs:970` | Error | Some | Subscribe forwarder: ability `Err` during `next_frame()`. | KEEP_DAEMON_INTERNAL |
| `src/services/control/ability_proxy.rs:992` | Terminal | Some | Subscribe forwarder: Terminal on stream complete/cancel. | KEEP_DAEMON_INTERNAL |
| `src/services/control/ability_proxy.rs:1041` | RecvBidi | Some | Bidi forwarder: RecvBidi per handler-output frame. | KEEP_DAEMON_INTERNAL |
| `src/services/control/ability_proxy.rs:1057` | ErrorBidi | Some | Bidi forwarder: ability `Err` during frame receive. | KEEP_DAEMON_INTERNAL |
| `src/services/control/ability_proxy.rs:1087` | TerminalBidi | Some | Bidi forwarder: exactly one TerminalBidi per session via `compare_exchange` idempotency guard (C-M3a §I2). | KEEP_DAEMON_INTERNAL |

**Notes.** These are the reception (`serve_connection`/`handle_request`/`send_booting_error`), dispatch (`handle_invoke`, `handle_subscribe_async`, `handle_bidi_open_async`), and forwarder (`spawn_forwarder`, `spawn_bidi_forwarder`) sides of the control plane. `subject` on Invoke/Subscribe/OpenBidi handlers is threaded *from the wire* into resolver→dispatcher via `EnvelopeContext`; `Result` carries the optional `receipt_header` (§A12). All error paths preserve `request_id`/`subscription_id`/`session_id` for client correlation. None are migration targets.

### 3.6 `test` (wire-format compatibility pins — out of scope for migration)

40 frame-construction sites across three modules. All `KEEP_CONTROL_PLANE_COMPAT`.

| Caller (file:line) | Frame(s) | Subject | Description | Verdict |
|---|---|---|---|---|
| `src/services/control/frames.rs:208` | Invoke | None | Invoke JSON round-trip. | KEEP_CONTROL_PLANE_COMPAT |
| `src/services/control/frames.rs:231` | Subscribe | Some | Subscribe round-trip with subject. | KEEP_CONTROL_PLANE_COMPAT |
| `src/services/control/frames.rs:273` | Result | N/A | Result request_id preservation. | KEEP_CONTROL_PLANE_COMPAT |
| `src/services/control/frames.rs:288` | Result | N/A | Result receipt_header omission. | KEEP_CONTROL_PLANE_COMPAT |
| `src/services/control/frames.rs:307` | OpenBidi | Some | OpenBidi round-trip with subject. | KEEP_CONTROL_PLANE_COMPAT |
| `src/services/control/frames.rs:345` | SendBidi | N/A | SendBidi round-trip. | KEEP_CONTROL_PLANE_COMPAT |
| `src/services/control/frames.rs:353` | CloseBidi | N/A | CloseBidi round-trip. | KEEP_CONTROL_PLANE_COMPAT |
| `src/services/control/frames.rs:368` | RecvBidi | N/A | RecvBidi session_id field. | KEEP_CONTROL_PLANE_COMPAT |
| `src/services/control/frames.rs:377` | TerminalBidi | N/A | TerminalBidi session_id field. | KEEP_CONTROL_PLANE_COMPAT |
| `src/services/control/frames.rs:386` | ErrorBidi | N/A | ErrorBidi session_id field. | KEEP_CONTROL_PLANE_COMPAT |
| `src/services/control/frames.rs:403` | Result | N/A | Result receipt_header emission. | KEEP_CONTROL_PLANE_COMPAT |
| `src/services/control/ability_proxy.rs:1251` | Invoke | None | invoke `system.ping` → Result, request_id preserved. | KEEP_CONTROL_PLANE_COMPAT |
| `src/services/control/ability_proxy.rs:1273` | Invoke | None | invoke unknown ability → NOT_FOUND. | KEEP_CONTROL_PLANE_COMPAT |
| `src/services/control/ability_proxy.rs:1299` | Cancel | N/A | cancel unknown id → error with subscription_id. | KEEP_CONTROL_PLANE_COMPAT |
| `src/services/control/ability_proxy.rs:1325` | Subscribe | None | subscribe session attach → Terminal at minimum. | KEEP_CONTROL_PLANE_COMPAT |
| `src/services/control/ability_proxy.rs:1387` | Invoke | None | observe.health attaches selfsigned header when host URA known. | KEEP_CONTROL_PLANE_COMPAT |
| `src/services/control/ability_proxy.rs:1437` | Invoke | None | consent ability attaches hosted_by header distinct from signer. | KEEP_CONTROL_PLANE_COMPAT |
| `src/services/control/ability_proxy.rs:1618` | OpenBidi | None | open/send/recv/close ordering (I1). | KEEP_CONTROL_PLANE_COMPAT |
| `src/services/control/ability_proxy.rs:1633` | SendBidi | N/A | send within open/send/recv/close. | KEEP_CONTROL_PLANE_COMPAT |
| `src/services/control/ability_proxy.rs:1646` | CloseBidi | N/A | close within open/send/recv/close. | KEEP_CONTROL_PLANE_COMPAT |
| `src/services/control/ability_proxy.rs:1692` | OpenBidi | Some | open bidi forwards subject into envelope context. | KEEP_CONTROL_PLANE_COMPAT |
| `src/services/control/ability_proxy.rs:1724` | OpenBidi | None | exactly one terminal (I2). | KEEP_CONTROL_PLANE_COMPAT |
| `src/services/control/ability_proxy.rs:1737` | CloseBidi | N/A | exactly one terminal (I2). | KEEP_CONTROL_PLANE_COMPAT |
| `src/services/control/ability_proxy.rs:1769` | OpenBidi | None | duplicate session_id → ErrorBidi, first session intact (D8). | KEEP_CONTROL_PLANE_COMPAT |
| `src/services/control/ability_proxy.rs:1784` | OpenBidi | None | duplicate session_id second open must error. | KEEP_CONTROL_PLANE_COMPAT |
| `src/services/control/ability_proxy.rs:1799` | SendBidi | N/A | duplicate session_id probe to first session. | KEEP_CONTROL_PLANE_COMPAT |
| `src/services/control/ability_proxy.rs:1851` | OpenBidi | None | open bidi unknown ability leaves no session state (I3). | KEEP_CONTROL_PLANE_COMPAT |
| `src/services/control/ability_proxy.rs:1875` | SendBidi | N/A | probe SendBidi for unknown session_id. | KEEP_CONTROL_PLANE_COMPAT |
| `src/services/control/ability_proxy.rs:1927` | SendBidi | N/A | send bidi for unknown session closes nothing (D5). | KEEP_CONTROL_PLANE_COMPAT |
| `src/services/control/ability_proxy.rs:1962` | CloseBidi | N/A | close bidi for unknown session is silent noop. | KEEP_CONTROL_PLANE_COMPAT |
| `src/services/control/ability_proxy.rs:1987` | OpenBidi | None | cancel token fires terminal with cancelled reason (D4). | KEEP_CONTROL_PLANE_COMPAT |
| `src/services/control/ability_proxy.rs:2032` | Invoke | None | frame omits receipt_header when local_agents_file empty. | KEEP_CONTROL_PLANE_COMPAT |
| `src/services/control/server.rs:555` | — | N/A | make_proxy wraps kernel handle (no frame construction). | KEEP_CONTROL_PLANE_COMPAT |
| `src/services/control/server.rs:571` | Invoke | None | booting state rejects invoke with BOOTING code. | KEEP_CONTROL_PLANE_COMPAT |
| `src/services/control/server.rs:604` | Subscribe | None | watch boot subscription receives ready terminal. | KEEP_CONTROL_PLANE_COMPAT |
| `src/services/control/server.rs:678` | Invoke | None | E2E smoke: invoke observe.health → Result. | KEEP_CONTROL_PLANE_COMPAT |
| `src/services/control/server.rs:800` | OpenBidi | None | E2E bidi: open echo session over wire codec. | KEEP_CONTROL_PLANE_COMPAT |
| `src/services/control/server.rs:811` | SendBidi | N/A | E2E bidi: send three frames over wire codec. | KEEP_CONTROL_PLANE_COMPAT |
| `src/services/control/server.rs:820` | CloseBidi | N/A | E2E bidi: close session over wire codec. | KEEP_CONTROL_PLANE_COMPAT |
| `src/services/control/server.rs:928` | Invoke | None | malformed-frame recovery: valid frame after bad JSON. | KEEP_CONTROL_PLANE_COMPAT |

**Notes.** These tests pin JSON schema round-trips, `request_id`/`subscription_id`/`session_id` correlation, subject propagation, receipt-header attachment, and bidi ordering invariants (I1–I3, D4–D8). They stay to guard wire-format stability for the boot/status/lifecycle traffic that survives demotion. They are **not** migration targets; whichever pin a frame type that is later removed from JSON control will be retired *with* that removal, not before.

---

## 4. Migration Sequencing

### Prerequisites (commit-plan Steps 3 & 4)

The four `MUST_MIGRATE` callers cannot move until the destination surface exists and is receipt-complete:

- **Step 3 — daemon SDK.** A first-class daemon-side SDK exposing the Invocation primitive (`InvocationClient` over `daemon.sock`, `src/support/local_daemon_grpc.rs:417`) with an ergonomic invoke/subscribe/cancel API that accepts and threads `subject` causal context. Until this exists, callers have nowhere correct to land.
- **Step 4 — complete-Invocation FFI.** The FFI layer must expose the Invocation primitive end-to-end (invoke / subscribe / cancel) *with* `subject` propagation from the foreign-language binding, so the FFI callers can move without re-introducing the receipt-chain gap (§1).

Neither the daemon SDK (Step 3) nor the complete-Invocation FFI (Step 4) may be assumed by this document; they are upstream gates.

### Migration order

1. **`src/facade/cli/groups/plugin.rs:216` (CLI plugin Invoke).** Lowest blast radius and a clean template: a single repo-local CLI command already adjacent to the gRPC path (`invoke_local_daemon_ability_with_subject`, `src/support/local_daemon_grpc.rs:346`). Re-home `device.plugin.reload` / `device.plugin.status` onto the Invocation primitive and supply a real `subject` (replacing the `:219` `None`). Verify: `easynet plugin` reload/status round-trips over `daemon.sock`; receipt carries causal context.
   *Prereq: Step 3.*

2. **`src/ffi/ability.rs:155` (FFI Invoke).** Move `easynet_ability_invoke` to the Invocation primitive; thread `subject` from the binding (replacing `:159` `None`). Verify against the FFI round-trip harness.
   *Prereq: Steps 3 + 4.*

3. **`src/ffi/ability.rs:461` (FFI Subscribe).** Move `easynet_ability_subscribe` streaming path; thread `subject` (replacing `:465` `None`).
   *Prereq: Steps 3 + 4.*

4. **`src/ffi/ability.rs:541` (FFI Cancel).** Move the cancellation path emitted by the streaming reader task to the Invocation primitive's cancel semantics. Sequenced last because it is coupled to the Subscribe lifecycle migrated in step 3.
   *Prereq: Steps 3 + 4; depends on step 3 above.*

5. **`src/ffi/client.rs:327` (FFI test harness) — UPDATE_AFTER_MIGRATION.** Once 2–4 land, re-point this round-trip test at the Invocation path so it exercises the production FFI surface. Keep it in the suite; do not delete.

### What does NOT block demotion

- `start_boot_watcher.rs:155` (`system.watch_boot`) — retained boot/status traffic (Step 6 keeps boot/status on control).
- All `daemon_internal` handler sites — receive/respond machinery, unaffected by caller migration.
- All `test` compat pins — guard the surviving wire format; retire only alongside the frame types they pin, if/when removed.

---

## 5. Demotion-Readiness Gate

JSON-control `Invoke` / `Subscribe` / `OpenBidi` may be removed **only when every box below is checked.** This is the explicit exit criterion for commit-plan Step 6.

- [ ] **G1 — Daemon SDK (Step 3) merged.** Invocation primitive exposed over `daemon.sock` with `subject` threading.
- [ ] **G2 — Complete-Invocation FFI (Step 4) merged.** FFI invoke/subscribe/cancel on the Invocation primitive, with `subject` propagated from the binding.
- [ ] **G3 — `src/facade/cli/groups/plugin.rs:216` migrated.** `device.plugin.*` no longer constructs `IncomingFrame::Invoke`; routes through the Invocation primitive with a real `subject`.
- [ ] **G4 — `src/ffi/ability.rs:155` migrated.** `easynet_ability_invoke` off JSON control; `subject` no longer `None`.
- [ ] **G5 — `src/ffi/ability.rs:461` migrated.** `easynet_ability_subscribe` off JSON control; `subject` no longer `None`.
- [ ] **G6 — `src/ffi/ability.rs:541` migrated.** FFI Cancel routed through Invocation cancel semantics.
- [ ] **G7 — `src/ffi/client.rs:327` updated.** Harness exercises the Invocation FFI path; still green.
- [ ] **G8 — Receipt-chain completeness.** Every migrated caller supplies a real `subject` (causal context); zero `subject: None` remain on any product-ability Invocation. (§1 completeness rule.)
- [ ] **G9 — Re-sweep returns empty.** Re-running the product-ability sweep over `src/ffi/` and `src/facade/cli/` yields **zero** product-ability `IncomingFrame::{Invoke,Subscribe,OpenBidi}` callers. Only the retained `system.watch_boot` Subscribe (`start_boot_watcher.rs:155`) and boot/status/lifecycle traffic remain.
- [ ] **G10 — Boot/status preserved.** `system.watch_boot` Subscribe and the `BootBus` forwarder (`src/services/control/server.rs:388-461`) still function; the boot-progress E2E test (`server.rs:604`) passes.
- [ ] **G11 — Compat tests reconciled.** Any `test` pin (§3.6) targeting a removed frame type is retired *in the same change* that removes it; all surviving pins green.
- [ ] **G12 — Baseline re-confirmed against `main`** (see §6).

When G1–G12 hold, `Invoke`/`Subscribe`/`OpenBidi` are safe to remove from the JSON control plane, leaving it scoped to boot/status/lifecycle as Step 6 requires.

---

## 6. Baseline Caveat

This inventory is **current-working-tree evidence**, captured on branch `codex/f07-json-control-caller-inventory-2026-06-04`. Every claim is traced to a `file:line` verified against the tree at authoring time (FFI callers at `src/ffi/ability.rs:155/461/541` with `subject: None` at `:159/:465`; CLI callers at `plugin.rs:216` and `start_boot_watcher.rs:155`; backend absence via `find`-confirmed zero `.go` files and zero `backend/` dirs; gRPC routing via `src/support/local_daemon_grpc.rs:417`).

Line numbers and caller populations **will drift** as Steps 3–6 land. Before any actual removal of JSON-control `Invoke`/`Subscribe`/`OpenBidi`:

1. **Re-run the six-category sweep against `main`** (not this feature branch) to confirm no new product-ability caller has been introduced and no cited line has moved.
2. **Re-confirm backend absence** (`find . -name '*.go'`, `find . -type d -name backend`) — a future backend integration could introduce a JSON caller this baseline does not anticipate.
3. **Treat G9 (empty re-sweep) as the authoritative gate**, not this static snapshot. This document is the migration *plan of record*; the re-sweep is the *go/no-go signal*.

Do not remove any frame variant on the strength of this document alone.
