# Verification

## Planned Checks

- `cargo test -q hosted_agent_name_lookup --lib`
- `cargo test -q hosted_agent_runtime --lib`
- `cargo test -q teach --lib`
- `cargo test -q canonical_hosted_agent_ura_by_name --lib --features axon-pb`
- `tools/scripts/check-architecture-convergence.sh`
- `bash tests/scripts/test_check_architecture_convergence.sh`
- `rustfmt --edition 2024 --check src/cli/commands/teach.rs src/daemon/axon_bridge/hot_agent_registrar.rs src/support/platform/local_daemon_grpc.rs`
- `git diff --check -- src/cli/commands/teach.rs src/daemon/axon_bridge/hot_agent_registrar.rs src/support/platform/local_daemon_grpc.rs pr/20260716-hosted-agent-name-aggregate-resolution`

## Results

- `cargo test -q hosted_agent_name_lookup --lib`: passed, 3 tests.
- `cargo test -q hosted_agent_runtime --lib`: passed, 2 tests.
- `cargo test -q teach --lib`: passed, 46 tests.
- `cargo test -q canonical_hosted_agent_ura_by_name --lib --features axon-pb`: passed compilation, 0 matching tests.
- `tools/scripts/check-architecture-convergence.sh`: passed.
- `bash tests/scripts/test_check_architecture_convergence.sh`: passed.
- `rustfmt --edition 2024 --check src/cli/commands/teach.rs src/daemon/axon_bridge/hot_agent_registrar.rs src/support/platform/local_daemon_grpc.rs`: passed.
- `git diff --check -- src/cli/commands/teach.rs src/daemon/axon_bridge/hot_agent_registrar.rs src/support/platform/local_daemon_grpc.rs pr/20260716-hosted-agent-name-aggregate-resolution`: passed.
