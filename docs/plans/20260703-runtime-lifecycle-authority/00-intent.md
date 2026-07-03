# Runtime Lifecycle Authority Intent

Goal: implement `docs/spec/runtime-lifecycle-authority-v1.md` without changing
the spec, while preserving the existing CLI command surface.

Implementation location: `src/daemon/boot/lifecycle/`. The runtime lifecycle
spec names `src/daemon/lifecycle/`, but `docs/spec/project-structure-v1.md`
and `tools/scripts/check-project-structure-v1.sh` are the active layout
authority and reject new top-level daemon directories. The `daemon` module
re-exports `boot::lifecycle` so callers still use the domain name without
violating the final tree.

Non-goals:

- Do not restore the retired heartbeat sidecar as a normal runtime stage.
- Do not reintroduce product-path `axon-runtime`.
- Do not infer product `ONLINE` from pid, sockets, or `runtime.json`.
- Do not alter Ability, Invocation, Receipt, URA, or Axon protocol semantics.

Acceptance criteria:

- Process facts drive status/start/stop before `runtime.json`.
- `runtime.json` is represented as `RuntimeSessionProjection`.
- Start attaches only to a matching daemon identity and refuses degraded
  half-alive endpoints.
- Stop plans from daemon facts even when projection is missing.
- Legacy Axon/heartbeat cleanup is explicitly named legacy.
- Project structure guard, lifecycle unit tests, and narrow cargo checks pass.
