# Mission `meta.json` Status Consumer Inventory

T5.3 precondition and closure note, refreshed on 2026-06-12.

## Inventory Result

Cross-repository consumers of mission run status are effectively absent:

- EasyNet backend, frontend, scripts, and integration tests do not directly parse the CLI mission `meta.json` status vocabulary.
- The daemon mission runtime owns the run directory layout in
  `src/daemon/execution/mission/orchestration.rs`; CLI commands are adapters.
- Other hits for `status`, `running`, or `mission_id` belong to unrelated surfaces such as device presence, boot status, control-plane diagnostics, and EAL trace metadata.

That collapses the compatibility boundary to one local requirement: historical mission `meta.json` files that store lowercase status strings must continue to deserialize.

## Implemented Shape

`src/daemon/execution/mission/orchestration.rs` owns the complete F-022/T5.3
state model:

- `MissionRunStatus` is an enum serialized with `#[serde(rename_all = "lowercase")]`.
- Historical literals `ok`, `error`, `partial`, `running`, and `cancelled` parse into the enum unchanged.
- Unknown status strings are rejected instead of flowing through as untyped state.
- `MissionRunMeta.status` is typed as `MissionRunStatus`, not `String`.
- `MissionRunStatus::is_terminal()` is the single terminal-state predicate.

Liveness no longer uses a pid file:

- `MissionRunStore::create` starts a run-owned heartbeat pump.
- A fresh `heartbeat` file means the run is alive now.
- A stale or missing heartbeat means `running = false`, even if `meta.status == Running`.
- `MissionRunSummary::is_interrupted()` identifies the exact crash state: stored `Running` plus dead heartbeat.
- `cancel_run` can settle an interrupted run to `Cancelled`.

## Test Evidence

The implementation is pinned by focused tests in
`src/daemon/execution/mission/orchestration.rs`:

- `status_serde_matches_historical_literals`
- `create_starts_heartbeat_and_finish_removes_it`
- `interrupted_run_reads_dead_not_running_forever`
- `cancel_run_flips_in_flight_to_cancelled`
- `cancel_run_noop_on_terminal`

## Boundary Decision

This remains EasyNet product-runtime persistence, not an Axon protocol object.
The daemon owns the local run directory and heartbeat lifecycle; CLI history
commands only project that state.

No backend or frontend migration is required for T5.3.
