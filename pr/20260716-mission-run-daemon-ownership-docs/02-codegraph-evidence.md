# CodeGraph-style evidence

Source evidence:

```text
src/daemon/execution/mission/orchestration.rs
```

- `MissionRunStore` anchors run directories under `~/.easynet/missions/runs/`.
- `MissionRunDir` owns the `HeartbeatPump`.
- `MissionRunStatus` serializes lowercase and contains `Running`, `Ok`,
  `Partial`, `Error`, and `Cancelled`.
- `MissionRunAggregate::apply_terminal` refuses to transition terminal runs.
- `run_mission_inproc` creates the run directory, installs
  `MissionContextGuard`, executes with `trace_id == run_id`, and records terminal
  metadata.

Docs checked before editing:

```sh
rg -n "mission_runs\\.rs|Completed|Failed|Aborted|MissionRunStatus" \
  docs/design/oop-design-handbook.md \
  docs/mission-run-status-consumer-inventory.md \
  docs/spec/seven-axes-p0-landing-v1.md
```

This found current owner updates plus stale residues:

- `docs/design/oop-design-handbook.md`: old `MissionRunStatus` terminal set.
- `docs/spec/seven-axes-p0-landing-v1.md`: old `mission_runs.rs`
  heartbeat/state-machine reference.
- `docs/spec/seven-axes-p0-landing-v1.md`: historical preface still pointed
  readers at the retired module instead of daemon orchestration.
