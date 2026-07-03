# Runtime Lifecycle Authority v1

Status: Active.

This SPEC defines the target authority model for `easynet start`,
`easynet stop`, `easynet runtime start`, `easynet runtime stop`, and
`easynet runtime status`.

It is a behavior SPEC, not a project-layout SPEC. The layout authority remains
[`project-structure-v1.md`](project-structure-v1.md). This file narrows one
behavioral gap left visible by the structure migration: daemon process facts,
CLI session projection, and legacy cleanup were mixed in command code. It also
separates local daemon lifecycle from product-layer online/offline visibility,
because the product does not learn "CLI is online" from `runtime.json`.

## 1. Problem Statement

The product runtime path is already conceptually unified: device mode and hub
mode start `easynet-daemon`, and `easynet-daemon` embeds the Axon
`LocalRuntime`. The product path must not require a standalone
`axon-runtime`.

The remaining problem is lifecycle authority:

- `~/.easynet/runtime.json` is treated by some CLI paths as if it were the
  runtime fact.
- `~/.easynet/easynet-daemon.pid`, `~/.easynet/control.json`, `control.sock`,
  and `daemon.sock` are the actual daemon discovery and liveness facts.
- `easynet runtime stop` has fallback process cleanup, but `status` and
  portions of `start` still reason from the projection first.
- Legacy cleanup for `AxonBridge` and the retired heartbeat sidecar is still
  rendered as if it were part of the normal lifecycle.

The result is a management split. It is not a product runtime split, but it is
enough to produce operator-visible dirty start/stop behavior.

## 2. Baseline Observations

Observed before the local lifecycle authority implementation in this branch:

1. Both top-level lifecycle aliases and layered lifecycle commands dispatch to
   the same implementation:
   `src/cli/mod.rs` forwards `Command::Start` and `Command::Stop`;
   `src/cli/commands/groups/runtime.rs` forwards `runtime start` and
   `runtime stop`.
2. Device start uses `DaemonStartConfig::device(...).start()` and then saves
   a `RuntimeState { runtime_kind: DaemonOnly, ... }`.
3. Hub start uses `DaemonStartConfig::hub().start()` and also saves
   `RuntimeKind::DaemonOnly`.
4. `easynet-daemon` owns the control socket, invocation socket,
   runtime-dispatch socket, pages listener, and `control.json`.
5. `status` first tried to load `runtime.json`; if it was absent, it reported
   `Runtime: not running`.
6. `stop` first derived its stop shape from `runtime.json`, but it also swept
   `easynet-daemon` by pidfile and `pgrep` when projection was missing.
7. The daemon entry point states that the legacy heartbeat sidecar was retired,
   but stop still has a visible `stop-heartbeat` stage.
8. The release E2E flow currently references a non-existent install harness
   path before it reaches the runtime start/stop assertions.
9. The heartbeat mechanism was migrated, not removed. The old sidecar is
   retired; the current device liveness model is the long-lived
   `session.open` bidi stream plus a session-lifetime `federation.heartbeat`
   loop inside the invocation transport.
10. Hub-side presence treats a device as online exactly while the admitted
    `session.open` stream is registered in `PresenceRegistry`. Stream close,
    reset, send failure, or admin revoke emits an `Offline` event.
11. `federation.subscribe_directory_v2` projects those presence events into
    directory `upsert` and `remove` frames for backend/SSE consumers. Its
    stream heartbeat is a subscriber keepalive, not device liveness.

These observations implied that command behavior mostly worked by layered
fallbacks, not by one explicit lifecycle authority. The local implementation in
this branch addresses that lifecycle authority split; backend/product presence
propagation remains a separate integration boundary.

## 3. Ownership Rules

### 3.1 Process Facts

Process facts are the authoritative local truth for daemon lifecycle:

- daemon pidfile: `~/.easynet/easynet-daemon.pid`
- discovery file: `~/.easynet/control.json`
- control endpoint: `control.sock` or Windows pipe
- invocation endpoint: `daemon.sock` or configured local endpoint
- accepting probes for control and invocation endpoints
- daemon identity from `control.json`: mode, realm, node id

Process facts answer: "Is a daemon process for this product identity actually
alive and callable?"

### 3.2 Session Projection

`~/.easynet/runtime.json` is a session projection. It is useful for operator
display, start metadata, and legacy compatibility. It is not the daemon fact.

The target name is:

```rust
RuntimeSessionProjection
```

It replaces the semantic role currently named `RuntimeState`.

Projection answers: "Which start command created or attached to this runtime,
and with which operator-facing metadata?"

Projection absence must not mean "daemon absent".

### 3.3 Legacy Cleanup

Legacy cleanup is a compatibility janitor:

- legacy `RuntimeKind::AxonBridge`
- stale `heartbeat.pid`
- stale old socket files
- stale old log paths

Legacy cleanup must be named as legacy. It must not appear as a normal product
daemon lifecycle stage.

### 3.4 Product Presence Facts

Product presence facts are the authoritative product truth for "does the
product know this device/agent is online?"

They are distinct from process facts:

- `session.open` admission and continued stream ownership in
  `PresenceRegistry`;
- `PresenceEvent::Online` and `PresenceEvent::Offline`;
- `DirectoryEvent` projection through `federation.subscribe_directory_v2`;
- `federation.heartbeat` refresh of `last_heartbeat_unix_ms` and
  owner-projection leases;
- backend directory subscription and poll/read-model reconciliation.

Product presence answers: "Can the Hub route to this URA and can the product
surface observe that state?"

Important constraints:

1. `federation.heartbeat` is a lease/directory refresh signal. It must not
   become the primary local daemon lifecycle fact.
2. `subscribe_directory` heartbeat frames prove only that the subscription
   stream is alive. They must not mark a device online by themselves.
3. `easynet start` may report local daemon readiness before product presence is
   online. A clean UI claim of `ONLINE` requires session admission or a fresh
   directory read showing active presence.
4. `easynet stop` may complete local process cleanup before every product
   subscriber has consumed the remove event. The bounded product guarantee is
   event delivery or heartbeat/transport timeout, not `runtime.json` removal.

## 4. Target Structure

The current project structure can support the fix cleanly by adding a daemon
lifecycle submodule:

```text
src/daemon/lifecycle/
  mod.rs
  service.rs
  start.rs
  stop.rs
  status.rs
  discovery.rs
  projection.rs
  errors.rs
```

Responsibilities:

| Module | Responsibility |
| --- | --- |
| `service.rs` | Public orchestration object, `RuntimeLifecycleService` |
| `start.rs` | Device and hub start workflows |
| `stop.rs` | Stop workflow and cleanup transaction |
| `status.rs` | Status workflow based on process facts first |
| `discovery.rs` | Read pidfile, `control.json`, socket probes, daemon identity |
| `projection.rs` | Read/write/remove `RuntimeSessionProjection` |
| `errors.rs` | Typed lifecycle errors |

CLI files stay thin:

```text
src/cli/commands/start.rs
src/cli/commands/stop.rs
src/cli/commands/status.rs
```

They parse command arguments, call `RuntimeLifecycleService`, and render the
returned report. They must not own daemon policy or lifecycle state machines.

### 4.1 Implemented Local Lifecycle Slice

This branch implements the local lifecycle authority slice inside
`src/daemon/lifecycle/`:

| Type | File | Purpose |
| --- | --- | --- |
| `DaemonDiscoverySnapshot` | `discovery.rs` | Daemon-owned process facts from pidfile, `control.json`, and endpoint probes |
| `RuntimeSessionProjection` | `projection.rs` | Wrapper around `runtime.json` so projection is not mistaken for authority |
| `RuntimeLifecycleStatus` / `RuntimeStatusReport` | `status.rs` | Pure classification of projection + daemon facts |
| `RuntimeStartPreflightAction` / `RuntimeStartPreflightReport` | `start.rs` | Start preflight decision and stale-projection cleanup |
| `RuntimeStopShape` / `RuntimeStopPlan` | `stop.rs` | Side-effect-free stop planning from lifecycle status |
| `RuntimeLifecycleService` | `service.rs` | Public facade for status, start preflight, stop plan, and projection commit rollback |
| `RuntimeLifecycleError` | `errors.rs` | Typed lifecycle boundary errors |

The current CLI integration is:

- `runtime status` renders `RuntimeStatusReport`; it no longer treats missing
  `runtime.json` as proof that the daemon is stopped.
- `runtime start` calls `RuntimeLifecycleService::preflight_start()` before
  spawning or attaching, and uses `save_projection_after_ready()` so failed
  projection persistence rolls back a daemon started by that command.
- `runtime stop` receives a `RuntimeStopPlan`, then executes the existing
  staged renderer. It now stops daemon facts discovered through `control.json`
  even when `runtime.json` is absent.

Product-presence observation remains a separate boundary because the backend
read model and SSE subscriber live outside this local CLI lifecycle layer.

## 5. Object Model

The service is an object composed from narrow I/O collaborators:

```rust
pub struct RuntimeLifecycleService<P, D, S, L> {
    preflight: P,
    daemon: D,
    projection_store: S,
    legacy_cleanup: L,
}

impl<P, D, S, L> RuntimeLifecycleService<P, D, S, L> {
    pub fn start(
        &self,
        request: StartRuntimeRequest,
    ) -> Result<StartRuntimeReport, RuntimeLifecycleError>;

    pub fn stop(
        &self,
        request: StopRuntimeRequest,
    ) -> Result<StopRuntimeReport, RuntimeLifecycleError>;

    pub fn status(
        &self,
    ) -> Result<RuntimeStatusReport, RuntimeLifecycleError>;
}
```

Traits are allowed only at real I/O seams:

- `DaemonProcess`: start, attach, stop, endpoint probes.
- `DaemonDiscovery`: pidfile, control discovery, socket probing.
- `RuntimeProjectionStore`: projection load/save/remove.
- `ProductPresenceObserver`: session admission and directory read probes.
- `LegacyRuntimeJanitor`: legacy axon/heartbeat cleanup.

Do not create traits for every helper. Three collaborators with real alternate
implementations justify a trait. Otherwise use concrete structs.

## 6. Required Types

### 6.1 Discovery Snapshot

```rust
pub struct DaemonDiscoverySnapshot {
    pub pid: Option<u32>,
    pub pid_alive: bool,
    pub control_accepting: bool,
    pub invocation_accepting: bool,
    pub discovery_identity: Option<DaemonIdentity>,
    pub endpoints: DaemonEndpoints,
}
```

Invariant 1: `control_accepting && invocation_accepting` is the minimum local
condition for a callable daemon.

Invariant 2: `discovery_identity` must match the requested mode, realm, and
node id before start attaches to an existing daemon.

Invariant 3: projection absence never changes these fact fields.

### 6.2 Runtime Session Projection

```rust
pub struct RuntimeSessionProjection {
    pub endpoint: String,
    pub process_kind: RuntimeProcessKind,
    pub pid: Option<u32>,
    pub hub: Option<String>,
    pub realm: Option<String>,
    pub label: Option<String>,
    pub started_at: Option<String>,
    pub credential_verified: Option<bool>,
}
```

`RuntimeProcessKind`:

```rust
pub enum RuntimeProcessKind {
    EasynetDaemon,
    LegacyAxonBridge,
}
```

### 6.3 Reports

Start, stop, and status return reports. Renderers consume reports.

Product presence is optional because hub-only mode and offline/local-only test
fixtures may not have a device session to observe.

```rust
pub struct StartRuntimeReport {
    pub attached_existing: bool,
    pub daemon: DaemonDiscoverySnapshot,
    pub product_presence: Option<ProductPresenceSnapshot>,
    pub projection_written: bool,
    pub pages_port: Option<u16>,
}

pub struct StopRuntimeReport {
    pub revoke: StageOutcome,
    pub daemon_stop: StageOutcome,
    pub legacy_cleanup: Vec<StageOutcome>,
    pub projection_removed: StageOutcome,
    pub remaining_daemons: Vec<u32>,
}

pub struct RuntimeStatusReport {
    pub daemon: DaemonDiscoverySnapshot,
    pub product_presence: Option<ProductPresenceSnapshot>,
    pub projection: Option<RuntimeSessionProjection>,
    pub status: RuntimeLifecycleStatus,
}
```

### 6.4 Product Presence Snapshot

```rust
pub struct ProductPresenceSnapshot {
    pub device_ura: Option<String>,
    pub session_admitted: bool,
    pub directory_status: Option<ProductPresenceStatus>,
    pub last_heartbeat_unix_ms: Option<i64>,
    pub dispatch_probe: Option<StageOutcome>,
}

pub enum ProductPresenceStatus {
    Online,
    Suspect,
    Draining,
    Removed,
    Unknown,
}
```

Invariant 1: `Online` requires admitted session presence or a fresh directory
record whose status maps to active. It is not inferred from pid, sockets, or
`runtime.json`.

Invariant 2: `Suspect` is a liveness doubt, not revocation. Missing heartbeat
or suspended directory status must not be rendered as a permanent removal.

Invariant 3: a product `Online` state should be accompanied by a successful
dispatch probe when the caller is about to expose an ability surface that
requires immediate routing.

## 7. Lifecycle State Machine

```text
Unknown
  -> ProjectionOnly
  -> ProcessDiscovered
  -> Starting
  -> ControlReady
  -> InvocationReady
  -> Running
  -> Stopping
  -> Stopped
```

Degraded states:

```text
ProjectionMissingProcessRunning
ProjectionPresentProcessMissing
ControlOnlyInvocationDown
IdentityMismatch
StartProjectionCommitFailed
StopTimedOut
LegacyCleanupFailed
```

`status` must surface degraded states. It must not collapse them into
"not running".

## 8. Happy Paths

### 8.1 Device Start

1. Load and verify device credentials.
2. Probe Hub session endpoint as a preflight.
3. Ensure device daemon config.
4. Build requested daemon identity: mode `device`, realm, node id.
5. Read `DaemonDiscoverySnapshot`.
6. If a matching daemon is already running, attach and continue.
7. If no daemon is running, start `easynet-daemon`.
8. Wait for control and invocation readiness.
9. Wait for daemon boot Ready.
10. Write `RuntimeSessionProjection`.
11. Return `StartRuntimeReport`.

Expected effect: repeated `easynet start` either refuses with a precise
matching-running report or attaches cleanly without starting a duplicate
daemon.

### 8.2 Hub Start

1. Validate hub daemon config and TLS requirements.
2. Build requested daemon identity: mode `hub`, realm.
3. Follow the same daemon discovery, start, readiness, and projection commit
   flow as device start.

Expected effect: hub and device lifecycle share one product daemon model.

### 8.3 Stop

1. Read `DaemonDiscoverySnapshot`.
2. Read optional `RuntimeSessionProjection`.
3. If projection and credentials support revoke, call revoke best-effort.
4. Stop the daemon by pidfile or attached process handle.
5. Confirm control and invocation endpoints are no longer accepting.
6. Run legacy cleanup.
7. Remove projection.
8. Return `StopRuntimeReport`.

Expected effect: after a successful stop, no matching `easynet-daemon` process
is alive, control and invocation endpoints reject probes, and projection is
removed.

### 8.4 Status

1. Read `DaemonDiscoverySnapshot`.
2. Read optional `RuntimeSessionProjection`.
3. Read optional `ProductPresenceSnapshot`.
4. Classify local runtime status from daemon facts first.
5. Add product presence and projection metadata as supplementary information.

Expected effect: a manually-started or SDK-started daemon without
`runtime.json` is reported as running with `ProjectionMissingProcessRunning`.
If device mode has a daemon but no admitted `session.open`, status reports
local daemon `Running` and product presence `Unknown` or `Suspect`, not a false
product `ONLINE`.

## 9. Bad Paths

### 9.1 Projection Commit Fails After Daemon Ready

Observed risk: current start writes `runtime.json` after daemon Ready. If that
write fails, the daemon remains alive but the CLI can later report "not
running".

Implemented local behavior:

- `RuntimeLifecycleService::save_projection_after_ready()` owns projection
  persistence after daemon Ready.
- If projection commit fails for a daemon started by this command, the service
  stops that daemon and reports `ProjectionPersistRolledBack`, or
  `ProjectionPersistRollbackFailed` if rollback itself fails.
- If the command attached to an existing daemon, it must not kill the daemon.
  It reports `ProjectionPersistFailed` and leaves daemon facts visible.

### 9.2 Projection Present, Process Missing

Target behavior:

- `status` reports `ProjectionPresentProcessMissing`.
- `start` may remove the stale projection only after confirming pid is not an
  EasyNet process and endpoints are not accepting.
- `stop` removes stale projection and reports no process stopped.

### 9.3 Process Running, Projection Missing

Target behavior:

- `status` reports `ProjectionMissingProcessRunning`.
- `stop` still stops the daemon by process facts.
- `start` attaches only if daemon identity matches the requested mode, realm,
  and node id.

### 9.4 Control Socket Up, Invocation Socket Down

Target behavior:

- `start` refuses to attach.
- `status` reports `ControlOnlyInvocationDown`.
- `stop` stops the daemon and cleans stale discovery.

### 9.5 PID Reuse

Target behavior:

- Every pidfile-driven kill must check that the PID belongs to an EasyNet
  process before signaling.
- Every stale-state start check must use process identity and endpoint probes,
  not `is_pid_alive` alone.

### 9.6 Legacy Axon Bridge Projection

Target behavior:

- `LegacyAxonBridge` is explicitly classified as legacy.
- Stop runs legacy cleanup.
- Start never creates a new legacy projection.

### 9.7 Stop Timeout

Target behavior:

- Stop returns `StopTimedOut` with the remaining PID and endpoint state.
- It does not remove projection as if the stop succeeded unless the projection
  is proven stale or the caller explicitly requests forced cleanup.

### 9.8 Daemon Running, Product Presence Missing

Observed risk: device-mode daemon can be locally ready while the product still
does not know it is online. This happens when hub endpoint or identity is
missing, `session.open` is still in backoff, admission failed, backend trust
bootstrap failed, or the backend directory subscriber is reconnecting.

Target behavior:

- `start` reports local daemon readiness separately from product presence.
- `status` surfaces product presence as `Unknown` or `Suspect` with the
  specific missing fact: no hub endpoint, no identity, session not admitted,
  heartbeat stale, directory unavailable, or dispatch probe failed.
- Product surfaces do not mark the device `ONLINE` from local daemon facts
  alone.

### 9.9 Product Shows Online After Local Stop

Observed risk: dirty stop or abrupt kill can leave product presence green for
some interval.

Target behavior:

- Graceful stop first gives the session path a chance to close and emit
  `PresenceEvent::Offline`.
- If the process is killed or the transport cannot deliver close, the hub-side
  transport/heartbeat timeout demotes the directory record.
- CLI stop reports local postconditions independently: process gone, endpoints
  down, projection removed. It must not claim every product subscriber has
  already consumed the remove event unless the observer confirms it.

### 9.10 Heartbeat Lease Lapse While Session Is Online

Observed risk: the product can see "owner online but requested ability NODATA"
when the owner-projection lease expires even though the session is still alive.

Target behavior:

- `federation.heartbeat` refreshes both device liveness metadata and published
  owner-projection leases.
- Presence and ability catalog projection are validated together for ability
  surfaces: online presence without routable advertised ability is degraded
  with a typed resolver state, not collapsed into generic offline.

## 10. What This Solves

This design solves:

1. False `not running` status when daemon is alive but projection is missing.
2. Duplicate daemon starts caused by stale or missing projection.
3. Dirty start after a failed start that left daemon facts behind.
4. Dirty stop that removes projection while leaving a callable daemon alive.
5. Confusing operator output where legacy heartbeat cleanup appears to be a
   normal current lifecycle stage.
6. Overbroad reasoning from `runtime.json` instead of daemon identity and
   endpoint facts.
7. Product UI saying offline after daemon start because `start` reported local
   readiness before `session.open` admission.
8. Product UI staying online after local stop because process cleanup and
   product presence propagation were treated as the same transaction.
9. False ability-missing states when heartbeat lease refresh and presence are
   not checked as separate projections.

It does not solve:

1. Bugs inside daemon shutdown handlers that keep tasks alive after SIGTERM.
2. OS-level signal delivery failures.
3. Permission failures that prevent unlinking socket or discovery files.
4. External processes started outside EasyNet that intentionally use the same
   socket paths.
5. Backend/UI bugs that ignore directory events or never re-poll after a
   subscriber reconnect gap.

Those become easier to diagnose because lifecycle reports will show the exact
remaining process facts.

## 11. Expected Effects

Operator-visible effects:

- `status` distinguishes clean stopped, running, projection missing, stale
  projection, control-only, and legacy states.
- `start` either starts one daemon, attaches to one matching daemon, or refuses
  with a precise mismatch.
- `stop` reports whether daemon stop, projection removal, and legacy cleanup
  each succeeded.
- `status` separates local daemon state from product presence state.
- CLI help and docs no longer say "Axon runtime" for product daemon start.

Engineering effects:

- CLI command files become presentation adapters.
- Lifecycle behavior becomes testable without spawning real processes.
- File/probe/process cleanup is centralized.
- Product presence checks become read-only observers rather than hidden start
  or stop side effects.
- Legacy cleanup is isolated and can be deleted later without touching the
  happy path.

## 12. Metrics and Acceptance Gates

### 12.1 Deterministic Lifecycle Matrix

A table-driven test must cover at least these input states:

| Projection | PID | Control | Invocation | Expected status |
| --- | --- | --- | --- | --- |
| absent | absent | down | down | `Stopped` |
| absent | alive | up | up | `ProjectionMissingProcessRunning` |
| present daemon | absent | down | down | `ProjectionPresentProcessMissing` |
| present daemon | alive | up | up | `Running` |
| present daemon | alive | up | down | `ControlOnlyInvocationDown` |
| present legacy | legacy pid | n/a | n/a | `LegacyAxonBridge` |

### 12.2 Repeated Start/Stop Cleanliness

Run 50 local start/stop cycles in a sandbox home.

Acceptance:

- 0 leftover matching `easynet-daemon` processes after final stop.
- 0 accepting `control.sock` probes after final stop.
- 0 accepting `daemon.sock` probes after final stop.
- 0 stale `easynet-daemon.pid` after final stop.
- 0 duplicate daemon process starts per cycle.

### 12.3 Failure Injection

Inject projection write failure after daemon Ready.

Acceptance:

- If daemon was newly spawned, no daemon remains alive.
- If daemon was pre-existing and attached, daemon remains alive.
- Report status is `StartProjectionCommitFailed`.

### 12.4 PID Reuse Guard

Seed pidfile with a live non-EasyNet PID.

Acceptance:

- Stop refuses to signal it.
- Start does not treat it as a valid running EasyNet daemon.
- Projection cleanup is explicit and reported.

### 12.5 Release Path Guard

Release E2E must reach runtime assertions.

Acceptance:

- `packaging/release/e2e-release-flow.sh` invokes the correct install harness.
- Release tarball does not ship `axon-runtime`.
- Runtime start does not change the `axon-runtime` process set.
- Runtime stop leaves no `easynet-daemon` process in the sandbox.

### 12.6 Product Presence Propagation

Run device start/stop against a hub with backend directory subscription enabled.

Acceptance:

- Start reaches local `InvocationReady` before product `Online`; both states
  are observable independently.
- After `session.open` admission, backend list/read-model reports `ONLINE`
  within 2 s on loopback and within the WAN E2E budget on latency-injected
  tests.
- Graceful stop produces a directory remove/SSE invalidation within 5 s on
  loopback.
- Abrupt kill produces product offline/removal within 20 s under the WAN E2E
  budget.
- `subscribe_directory` heartbeat frames never change device online state by
  themselves.

### 12.7 Heartbeat Lease Renewal

Run an ability invoke immediately after join and again after the
owner-projection lease TTL has elapsed.

Acceptance:

- The second invoke succeeds while the device session remains admitted.
- No `NODATA: owner is online but does not publish the requested ability`
  response appears solely because the heartbeat lease was not refreshed.
- Heartbeat failures are visible as `Suspect` or resolver-state degradation,
  not silently rendered as `ONLINE`.

### 12.8 Current Regression Coverage

The current branch has regression coverage for two narrow claims:

1. `status_classifier_detects_projection_missing_live_daemon` is the
   red-before-green lifecycle-authority test. It constructs the exact broken
   state: no `runtime.json`, but daemon discovery/process facts are present.
   Passing means status classification no longer collapses this state into
   `Stopped`.
2. `json_payload_exposes_projection_missing_daemon_state` pins the operator/API
   surface: `runtime_status` must be `projection_missing_process_running`,
   `runtime` may be null, and daemon facts must still be visible.
3. `status_classifier_keeps_projection_only_as_degraded_not_running` pins the
   opposite stale state: `runtime.json` alone is not enough to claim a running
   daemon.
4. `start_preflight_attaches_when_projection_is_missing_but_daemon_is_live`
   pins the start behavior for the same migration state: start must attach and
   rebuild projection instead of starting a duplicate daemon.
5. `start_preflight_refuses_control_only_daemon` pins the broken half-alive
   state: control-only daemon is not attachable for product start.
6. `stop_plan_treats_projection_missing_live_daemon_as_daemon_only` pins the
   stop behavior for missing projection plus live daemon facts.
7. `stop_plan_preserves_legacy_axon_runtime_shape_from_projection` keeps legacy
   raw Axon bridge cleanup visible as a separate stop shape.
8. `supervisor_reconnects_when_hub_starts_after_cli_daemon` simulates a
   device-mode daemon whose Hub endpoint is initially down, then starts a fake
   Hub on the same endpoint. Passing means the device-side session supervisor
   keeps running after the first connect failure and creates `session.open`
   when the Hub appears.
9. `invoke_stream_subscribe_directory_v2_emits_heartbeat_when_idle` asserts
   that a directory-stream heartbeat does not create any presence row. Passing
   means subscriber keepalive is not currently being treated as device online
   state in that daemon stream test.

The first seven tests are bug detectors for the local lifecycle split. The last
two tests narrow the product-presence suspicion by proving that two lower-level
session/directory seams are behaving as intended.

These tests still do not prove the full product path is clean. They do not cover:

- real `easynet start` / `easynet stop` process cleanup;
- backend SSE subscriber reconnect gaps;
- backend list/read-model polling after daemon reconnect;
- dirty stop leaving an old daemon/session alive.

Therefore their result narrows the suspected fault: the "Hub starts after CLI"
session-supervisor path is probably not the failing seam. Remaining likely
seams are lifecycle projection cleanup and backend/product read-model
projection.

## 13. Is Dirty Start/Stop Caused By This?

Likely yes for a meaningful subset of cases.

The current architecture can produce dirty lifecycle states when process facts
and projection disagree:

- daemon Ready but `runtime.json` missing;
- `runtime.json` present but daemon dead;
- pidfile present but PID reused;
- control socket accepting but invocation socket down;
- stop removes projection while daemon shutdown times out;
- stateless stop finds a daemon only through fallback sweep.

Those are exactly the states a lifecycle authority layer would classify and
handle deterministically.

This is not proof that every dirty cleanup incident is caused by the projection
split. Some dirty states can come from daemon shutdown bugs, socket unlink
permission failures, task leaks, or OS signal behavior. The fix is still the
right first move because it makes those causes observable instead of
collapsing them into "start failed" or "not running".

## 14. Is Product Online/Offline Drift Caused By This?

Likely yes for a related subset of cases, but the precise cause is product
presence authority drift rather than heartbeat deletion.

The migration intentionally retired the legacy heartbeat sidecar and moved
liveness into the invocation transport. The current model still has heartbeat:

- `session.open` is the canonical product liveness fact;
- session up-heartbeat keeps the bidi up stream active and detects send
  failure;
- `federation.heartbeat` refreshes directory heartbeat fields and
  owner-projection leases;
- `subscribe_directory_v2` heartbeats keep subscriber streams alive.

The product can still miss online/offline transitions when code observes the
wrong layer:

1. Local daemon is running, but `session.open` was not started or admitted.
2. `start` returns local readiness while session admission is still pending.
3. `stop` kills or times out the process before the hub observes stream close.
4. A stale daemon/session survives local cleanup and the product correctly
   still sees an online session.
5. Backend directory subscription is unavailable, lagged, or waiting for
   reconnect and the UI depends on polling fallback.
6. Heartbeat lease refresh fails, so ability projection expires while session
   presence still looks online.

Therefore dirty local start/stop and product online/offline drift are two
faces of the same structural problem: commands and product surfaces need one
explicit authority per fact type. The fix is not to restore the sidecar. The
fix is to make lifecycle reports and product observers distinguish local
daemon facts, session presence facts, heartbeat lease facts, and backend
read-model facts.

## 15. Happy Path And Bad Path For Product Presence

### 15.1 Product Online Happy Path

1. `easynet start` starts or attaches to one matching `easynet-daemon`.
2. Control and invocation endpoints accept probes.
3. Device mode starts the session supervisor.
4. Hub admits `session.open`.
5. Hub `PresenceRegistry` emits `Online`.
6. Directory v2 emits `upsert`.
7. Backend broker invalidates devices/agents through SSE.
8. Product list/read-model returns `ONLINE`.

### 15.2 Product Offline Happy Path

1. `easynet stop` discovers the local daemon by process facts.
2. Stop initiates graceful shutdown.
3. Session closes or the hub removes the session on stream close/reset.
4. Hub `PresenceRegistry` emits `Offline`.
5. Directory v2 emits `remove`.
6. Backend broker invalidates devices/agents through SSE.
7. Product list/read-model returns `UNKNOWN`, `SUSPECT`, or explicit removal
   according to backend device-state policy.
8. Local postconditions show no daemon process and no accepting endpoints.

### 15.3 Product Presence Bad Paths

- Daemon ready but no hub endpoint or identity: local `Running`, product
  `Unknown`.
- Session reconnect loop: local `Running`, product `Suspect` or `Unknown`.
- Backend subscriber lagged: product polling must re-read directory snapshot.
- Heartbeat refresh failing: presence may be online, but ability projection
  must degrade instead of returning stale `ONLINE`.
- Abrupt kill: product offline is bounded by transport/heartbeat timeout, not
  by immediate local pid disappearance.
- Dirty stop leaves old daemon alive: product may correctly stay `ONLINE`,
  proving local cleanup failed rather than product presence failed.

## 16. Implementation State

Implemented in this branch:

1. Introduced `daemon/lifecycle` data types and fake-based tests.
2. Implemented fact-first `status`.
3. Implemented start preflight from lifecycle facts.
4. Implemented transactional projection commit after daemon Ready.
5. Implemented fact-first stop planning.
6. Added discovery-PID daemon shutdown and stale `control.json` cleanup to
   `runtime stop`.
7. Moved lifecycle classification out of CLI command files.
8. Added hub-after-CLI reconnect and directory heartbeat regression tests.

Remaining outside the local lifecycle slice:

1. Release E2E install harness path so packaged start/stop assertions run.
2. Full real-process start/stop cycle test in a sandbox home.
3. Backend/product read-model observer and SSE reconnect coverage.
4. Product presence propagation tests for graceful stop, abrupt kill, backend
   subscriber reconnect, and heartbeat lease renewal.
5. Optional extraction of legacy raw Axon cleanup side effects behind a
   dedicated janitor object if that path stays supported.

## 17. Non-Goals

- Do not reintroduce raw `axon-runtime` into product start.
- Do not move EasyNet daemon lifecycle into Axon SDK.
- Do not make JSON control frames a product ability call surface.
- Do not stabilize new one-method-per-ability FFI.
- Do not hide daemon auto-start behind normal Invocation calls.
- Do not reintroduce the legacy heartbeat sidecar as the product liveness
  authority.
