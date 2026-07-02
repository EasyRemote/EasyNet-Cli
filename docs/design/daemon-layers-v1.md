# Daemon Layers v1 — Kernel-like Runtime Boundary

> Plan v10.1–v10.2. Describes the single-daemon architecture (scheme
> X), the three internal layers, the two hard trait boundaries, and
> the v1 explicit non-guarantees.

## 1. Process topology

One device = one Axon node = one `easynet-daemon` process. The
`easynet` bin is a thin user CLI that either connects to the daemon
over local IPC (for operations that need runtime state) or runs
self-contained (for stateless operations like `easynet skill
install`).

```
Client GUI
   │
   │  FFI: libeasynet_cli.{so,dylib,dll,a} + easynet_cli.h
   ▼
lib (cdylib)
   │
   │  Local IPC:
   │    Unix/macOS  — UDS  ~/.easynet/control.sock  (mode 0600)
   │    Windows     — Named Pipe \\.\pipe\easynet-<uid>
   │    Auth = filesystem permissions; no bearer token.
   ▼
easynet-daemon (single process)
   ├──► loopback handler (local ability)
   └──► Axon a2a (remote ability)
                     ▼
            easynet-daemon on peer
```

## 2. Internal layering

Inside `easynet-daemon`:

```
[userspace (Client)]      via FFI → libeasynet_cli (cdylib)
    ↓
[libc-equivalent]        IPC client (in lib, not shown)
    ↓
[Control]                src/daemon/control/              (local boot/status IPC)
    ↓  (only via KernelApi trait)
[KernelApi]              src/runtime/kernel_api.rs        ← SYSCALL BOUNDARY
    ↓  (trait impl on the Kernel)
[Execution]              src/runtime/execution/
    ├── session/         one session per agent run         (PR-ATTACH)
    ├── permission/      broker + pending queue            (PR-PERM)
    ├── discuss/         multi-agent room store            (PR-DISCUSS)
    ├── schedule/        cron store + tick runner          (PR-SCHED)
    └── loop_instance/   EAL loop wrapper store            (PR-LOOP)
    ↓  (only via GatewayApi trait)
[GatewayApi]             src/runtime/gateway_api.rs       ← NETWORK BOUNDARY
    ↓
[Gateway]                src/runtime/gateway.rs           (holds DendriteBridge)
```

### Two hard trait boundaries

The layering is enforced at CI time:

- `engineering/scripts/check-kernel-boundary.sh` — Control layer may only
  import syscall-boundary runtime modules plus stable ability-name constants
  from `crate::runtime`. Anything else is rejected.
- `engineering/scripts/check-kernel-boundary.sh` (rule 2) — Daemon Invocation
  transport may import the explicitly listed runtime adapters it
  needs to translate Axon `Invocation` frames into daemon-local
  execution. The allowed names are the current semantic owners:
  ability model/descriptors/names, system abilities, system ability catalog,
  hosted-agent ability specs, local invocation identity, owner projection,
  resources, failure-code projection, keyring, publish/advertise, federation
  helpers, Axon bridge, execution handles, ability wire metadata, and
  plugin-host runtime handles. `ability_wire` is allowed only as a metadata
  boundary: the transport reads the local bidi codec profile from
  `AbilityWireRegistry`, but plugin package ownership and execution policy
  remain in `runtime/plugin_host` and the Axon `LocalRuntime`. Plugin packages
  contribute `AbilityImpl` bindings; `DaemonPluginBinder` applies daemon
  authority policy before writing them into `AxonAbilityCatalog`. Resource,
  permission, and realtime transport readiness are projected by daemon-owned
  plugin-host brokers/adapters; plugin packages do not own policy,
  caller/callee identity, or transport admission. `plugin_host` itself is
  allowed only for the boot-injected `PluginRuntimeManager` handle used to
  execute already-loaded plugin-backed abilities; install/load policy stays in
  `runtime/plugin_host`. See `docs/design/plugin-contribution-boundary.md`.
- `engineering/scripts/check-kernel-boundary.sh` (rule 3) — Execution may
  only touch the network via `crate::runtime::gateway_api`, not
  the concrete `runtime::gateway`.
- `engineering/scripts/check-subservice-isolation.sh` — Execution
  sub-services cannot import each other.
- `engineering/scripts/check-invocation-unity.sh` — IPC/Kernel/Gateway
  method signatures must not speak `args_json`;
  `Kernel::invoke` cannot be called from inside an Execution
  sub-service.
- `engineering/scripts/check-dispatch-boundary.sh` — ability handlers under
  `runtime/system/` cannot branch on node identity.

## 3. Scheme X justification — one daemon, not two

The plan rejected scheme Y (daemon + separate "network gateway"
process) for three reasons documented in the plan itself:

1. `DendriteBridge` is hard to share across processes; single
   owner avoids a whole category of bug.
2. Session / schedule / permission pending state is easier to
   reason about in one address space.
3. A Client dying must not take down the daemon; the existing
   `run_daemon` already provides that separation. Splitting the
   daemon itself buys nothing.

## 4. Execution sub-service model (v10.2)

Each sub-service owns one slice of state:

| sub-service     | state                                       | PR         |
|-----------------|---------------------------------------------|------------|
| session/        | live-session index + timeline broadcast     | PR-ATTACH  |
| permission/     | broker trait + pending queue                | PR-PERM    |
| discuss/        | room registry + per-room broadcast          | PR-DISCUSS |
| schedule/       | JSON-file-backed cron + tick runner         | PR-SCHED   |
| loop_instance/  | loop-instance registry + status store       | PR-LOOP    |

The Kernel holds one handle per sub-service:

```rust
pub struct Kernel {
    session: SessionService,
    permission: PermissionService,
    discuss: DiscussService,
    schedule: ScheduleService,
    loop_svc: LoopService,
    gateway: Arc<dyn GatewayApi>,
}
```

Sub-services talking to each other go through the Kernel; they
never import each other's modules.

## 5. What v1 explicitly does not guarantee

These are declared up front so a future reviewer does not
mis-attribute a missing feature to an oversight.

- **No isolation model.** Sub-services share the same tokio
  runtime, thread pool, and heap. Panic is bounded by task
  boundary, which is not strong isolation.
- **No scheduling policy.** tokio's fair-ish task scheduling is
  the full extent of v1. No fairness between tenants (there is
  only one tenant in v1 anyway), no priority, no quota.
- **No resource accounting.** Sub-services do not record per-
  ability / per-agent CPU / memory / token usage.
- **No permission domain.** There is no "user A may invoke
  schedule; user B may not" concept. v1 is single-tenant;
  permission is interactive approval, not domain security.

v2 extension points:
- scheduler layer between `Kernel::invoke` admission and
  dispatch (see `docs/design/invocation-unity-v1.md` §6)
- resource accounting as sub-service hooks
- process-level isolation (cgroup / namespace) only lands in
  v3 / containerised deployments

## 6. Future planner position

A future `easynet-planner` process (or a module in-process) sits
*on top of* the KernelApi, peer with the Control layer. It
consumes `AbilityDescriptor`, `Session`, `InvocationRecord`, etc.
and produces Invocation plans that are then routed back through
`Kernel::invoke`. The minimum trait the planner needs is frozen
in `docs/design/planner-interface-v1.md`.

The planner is out of scope for this plan. What this plan delivers
is the trait-surface invariant that lets the planner land
additively later.
