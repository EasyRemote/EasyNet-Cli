# Verification

## Planned Checks

- `cargo test -q agent_aggregate --lib`
- `cargo test -q chat --lib`
- `tools/scripts/check-architecture-convergence.sh`
- `bash tests/scripts/test_check_architecture_convergence.sh`
- `rustfmt --edition 2024 --check src/daemon/persistence/agent_aggregate.rs src/daemon/persistence/local_agents.rs src/daemon/ability/builtins/governance/teach.rs src/daemon/ability/builtins/agents/chat.rs`
- `git diff --check -- src/daemon/persistence/agent_aggregate.rs src/daemon/persistence/local_agents.rs src/daemon/ability/builtins/governance/teach.rs src/daemon/ability/builtins/agents/chat.rs tools/scripts/check-architecture-convergence.sh tests/scripts/test_check_architecture_convergence.sh pr/20260716-teach-governance-agent-aggregate-identity pr/20260716-agent-chat-aggregate-provider`

## Results

- `cargo test -q agent_aggregate --lib`: 13 passed.
- `cargo test -q chat --lib`: 109 passed.
- `tools/scripts/check-architecture-convergence.sh`: passed.
- `bash tests/scripts/test_check_architecture_convergence.sh`: passed.
- Scoped rustfmt check for changed Rust files with edition 2024: passed.
- Full `cargo fmt --check`: not used as an acceptance gate for this slice because unrelated dirty files in the existing worktree require formatting.
