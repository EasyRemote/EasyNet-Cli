# Mission run daemon ownership docs

## Intent

Converge mission-run architecture documentation on the current implementation:
the daemon mission orchestration service owns run persistence, heartbeat
liveness, trace anchoring, and terminal status projection; CLI commands are
thin adapters.

## Expected effect

- Architecture convergence: one owner is documented for mission-run lifecycle.
- Product consistency: status vocabulary in docs matches the persisted enum.
- Future work clarity: watch/TUI and mission history readers depend on daemon
  mission-run state, not a retired CLI-local `mission_runs.rs` module.
