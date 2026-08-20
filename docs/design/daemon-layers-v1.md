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
[KernelApi]              src/daemon/boot/kernel/api.rs    ← SYSCALL BOUNDARY
    ↓  (trait impl on the Kernel)
[Execution]              src/daemon/execution/
    ├── session/         one session per agent run         (PR-ATTACH)
    ├── permission/      broker + pending queue            (PR-PERM)
    ├── mission/         agent mission drivers/discuss     (PR-DISCUSS)
    ├── pty/             PTY-backed execution resources
    ├── mcp/             daemon MCP execution bridge
    ├── schedule/        cron store + tick runner          (PR-SCHED)
    └── loop_instance/   EAL loop wrapper store            (PR-LOOP)
```

### Execution boundary

The layering is enforced at CI time:

- `tools/scripts/check-kernel-boundary.sh` — final-forbidden source roots
  (`src/runtime`, `src/services`, `src/facade`, `src/persistence`,
  `src/plugins`, `src/registry`) and their crate-root namespaces must not
  return. Daemon control/invocation production code must not depend on
  CLI/FFI edge modules. The retired `src/daemon/kernel` root must not
  return; the supported kernel home is `src/daemon/boot/kernel`.
- `tools/scripts/check-kernel-boundary.sh` (rule 4) — Execution does not
  import federation transports. Network publication and remote calls belong
  to daemon Invocation/session ownership.
- `tools/scripts/check-subservice-isolation.sh` — Execution
  sub-services cannot import each other.
- `tools/scripts/check-invocation-unity.sh` — the old
  `crate::daemon::kernel` namespace must not return; execution
  sub-services cannot bypass daemon invocation dispatch through legacy
  mission/session paths.
- `tools/scripts/check-dispatch-boundary.sh` — ability handlers under
  `src/daemon/ability/builtins/` cannot branch on node identity.

## 3. Scheme X justification — one daemon, not two

The plan rejected scheme Y (daemon + separate "network gateway"
process) for three reasons documented in the plan itself:

1. Invocation transport and session membership have one daemon owner.
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
| mission/        | mission drivers + discuss room store        | PR-DISCUSS |
| pty/            | PTY resources                               |            |
| mcp/            | MCP execution bridge                        |            |
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
- scheduler layer between `daemon::boot::kernel::Kernel::invoke` admission and
  dispatch (see `docs/design/invocation-unity-v1.md` §6)
- resource accounting as sub-service hooks
- process-level isolation (cgroup / namespace) only lands in
  v3 / containerised deployments

## 6. Future planner position

A future `easynet-planner` process (or a module in-process) sits
*on top of* the KernelApi, peer with the Control layer. It
consumes `AbilityDescriptor`, `Session`, `InvocationRecord`, etc.
and produces Invocation plans that are then routed back through
`daemon::boot::kernel::Kernel::invoke`. The minimum trait the planner needs is frozen
in `docs/design/planner-interface-v1.md`.

The planner is out of scope for this plan. What this plan delivers
is the trait-surface invariant that lets the planner land
additively later.
