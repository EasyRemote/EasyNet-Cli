# Task Plan Pack — Loop controller self-drive

Date: 2026-04-25
Scope: `EasyNet-Cli` daemon-side `system.loop.*` runtime

## Objectives

1. Make `system.loop.create` produce a loop that advances without external nudging.
2. Bring `LoopInstance` closer to the documented public contract by persisting loop config and latest observable outputs.
3. Expose a real loop status stream (`snapshot + live`) so Client bindings see progress instead of a frozen `pending`.
4. Keep loop execution inside the daemon boundary and bounded by `max_iters`; do not turn loop into a planner.

## Non-negotiable invariants

### Semantic invariants

1. Every created loop reaches a deterministic terminal state:
   - `done`
   - `exhausted`
   - `verify_malformed`
   - `cancelled`
2. `system.loop.status` must report the persisted loop config and last observable body/verify outputs.
3. A daemon restart must not erase loop metadata or make `status` lie about the latest persisted state.
4. A cancelled or already-terminal loop must never re-enter `running`.

### Concurrency and boundedness invariants

1. At most one controller task may actively drive a given `loop_id` at a time.
2. Each iteration is strictly ordered:
   - body invocation
   - verify evaluation
   - terminal/next-iteration decision
3. Loop progress is hard-bounded by `max_iters`; no unbounded retry path is allowed.
4. Live subscribers may lag, but lag must not corrupt loop state or crash the daemon.

### Layering invariants

1. Loop controller logic stays in `runtime/execution/loop_instance/*`; ability handlers remain thin.
2. Daemon boot wires controller startup; `system.loop.create` only registers and kicks the controller.
3. Loop remains a local control primitive, not a planner or cross-node orchestrator.

## Planned execution

1. Extend loop domain/store with missing config and observable outputs.
2. Add per-loop broadcast + controller bookkeeping to `LoopService`.
3. Implement controller task launch on create and on daemon boot for recoverable in-flight loops.
4. Update `system.loop.{create,status,subscribe,cancel}` to use the richer service surface.
5. Run focused Rust tests, then rerun the Node daemon-backed streaming probe from `EasyNet-Client`.
