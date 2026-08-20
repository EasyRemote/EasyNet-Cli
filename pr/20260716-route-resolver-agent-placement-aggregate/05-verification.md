# Verification

## Planned Checks

- `cargo test -q hosted_agent_placements --lib`
- `cargo test -q hosted_agent_placements_project_valid_agent_hosts --lib`
- `cargo test -q hosted_agent_placements_fail_closed_without_host_device --lib`
- `cargo test -q hosted_agent_placements_consume_aggregate_projection --lib`
- `cargo test -q hosted_agent_placements_unavailable_fails_closed --lib`
- `tools/scripts/check-architecture-convergence.sh`
- `bash tests/scripts/test_check_architecture_convergence.sh`
- `git diff --check -- src/daemon/invocation/routing/route_resolver.rs src/daemon/persistence/agent_aggregate.rs tools/scripts/check-architecture-convergence.sh tests/scripts/test_check_architecture_convergence.sh pr/20260716-route-resolver-agent-placement-aggregate`

## Results

- `cargo test -q hosted_agent_placements --lib`: 4 passed.
- `cargo test -q hosted_agent_placements_project_valid_agent_hosts --lib`: 1 passed.
- `cargo test -q hosted_agent_placements_fail_closed_without_host_device --lib`: 1 passed.
- `cargo test -q hosted_agent_placements_consume_aggregate_projection --lib`: 1 passed.
- `cargo test -q hosted_agent_placements_unavailable_fails_closed --lib`: 1 passed.
- `tools/scripts/check-architecture-convergence.sh`: passed.
- `bash tests/scripts/test_check_architecture_convergence.sh`: passed.
- Scoped `git diff --check`: passed.
