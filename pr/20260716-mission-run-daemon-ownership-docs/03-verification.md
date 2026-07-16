# Verification

Executed checks:

```sh
rg -n "mission_runs\\.rs|MissionRunStatus.*Completed|MissionRunStatus.*Failed|MissionRunStatus.*Aborted" \
  docs/design/oop-design-handbook.md \
  docs/mission-run-status-consumer-inventory.md \
  docs/spec/seven-axes-p0-landing-v1.md
```

Result: no matches. `rg` returned `1`, which is the expected status for an
empty match set.

```sh
cargo test -p easynet status_serde_matches_historical_literals
cargo test -p easynet create_starts_heartbeat_and_finish_removes_it
cargo test -p easynet interrupted_run_reads_dead_not_running_forever
cargo test -p easynet cancel_run_flips_in_flight_to_cancelled
bash tools/scripts/check-architecture-convergence.sh
bash tests/scripts/test_check_architecture_convergence.sh
```

Result: all passed.

```sh
git diff --cached --check
```

Result: run after staging.
