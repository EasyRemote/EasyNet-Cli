# Verification

## Planned Checks

- `cargo test -q hosted_agent_identity --lib`
- `cargo test -q agent_aggregate --lib`
- `cargo test -q local_agents --lib`
- `cargo test -q teach --lib`
- `tools/scripts/check-architecture-convergence.sh`
- `bash tests/scripts/test_check_architecture_convergence.sh`
- `rustfmt --edition 2024 --check src/daemon/persistence/agent_aggregate.rs src/daemon/persistence/local_agents.rs src/daemon/ability/builtins/governance/teach.rs src/daemon/ability/builtins/agents/chat.rs`
- `git diff --check -- src/daemon/persistence/agent_aggregate.rs src/daemon/persistence/local_agents.rs src/daemon/ability/builtins/governance/teach.rs src/daemon/ability/builtins/agents/chat.rs tools/scripts/check-architecture-convergence.sh tests/scripts/test_check_architecture_convergence.sh pr/20260716-teach-governance-agent-aggregate-identity pr/20260716-agent-chat-aggregate-provider`

## Results

- `cargo test -q hosted_agent_identity --lib`: 3 passed.
- `cargo test -q agent_aggregate --lib`: 13 passed.
- `cargo test -q local_agents --lib`: 11 passed.
- `cargo test -q teach --lib`: 46 passed.
- `tools/scripts/check-architecture-convergence.sh`: passed.
- `bash tests/scripts/test_check_architecture_convergence.sh`: passed.
- `rustfmt --edition 2024 --check src/daemon/persistence/agent_aggregate.rs src/daemon/persistence/local_agents.rs src/daemon/ability/builtins/governance/teach.rs src/daemon/ability/builtins/agents/chat.rs`: passed.
- Scoped `git diff --check`: passed.
