# Verification

## Evidence

- `rustfmt --edition 2021 src/cli/commands/start.rs src/daemon/ability/builtins/agents/lifecycle.rs`
- `rustfmt --edition 2021 --check src/cli/commands/start.rs src/daemon/ability/builtins/agents/lifecycle.rs`
- `cargo test --features axon-pb startup_bootstrap_projection_persists_through_lifecycle_owner --lib`
- `cargo test --features axon-pb agent_lifecycle --lib`
- `tools/scripts/check-architecture-convergence.sh`
- `tests/scripts/test_check_architecture_convergence.sh`
- `git diff --check -- src/cli/commands/start.rs src/daemon/ability/builtins/agents/lifecycle.rs tools/scripts/check-architecture-convergence.sh tests/scripts/test_check_architecture_convergence.sh pr/20260716-agent-bootstrap-lifecycle-owner`

## Result

- `cli start` no longer loads and saves `local-agents.json` directly.
- Startup hosted identity projection is now a lifecycle operation guarded by
  `AgentLifecycleMutationGuard`.
- R22 now rejects both lifecycle-internal projection-store bypass and CLI
  startup direct identity writes.
