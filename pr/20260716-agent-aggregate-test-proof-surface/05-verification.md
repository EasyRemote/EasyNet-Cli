# Verification Plan

```bash
cargo test agent_aggregate --lib
cargo check --features axon-pb --lib --all-targets 2>&1 | tee target/agent-aggregate-test-proof-surface-check.log
rg 'AgentAggregateSnapshot|HostedAgentIdentityProjection' target/agent-aggregate-test-proof-surface-check.log
bash tools/scripts/check-architecture-convergence.sh
bash tests/scripts/test_check_architecture_convergence.sh
git diff --check -- src/daemon/persistence/agent_aggregate.rs tools/scripts/check-architecture-convergence.sh tests/scripts/test_check_architecture_convergence.sh pr/20260716-agent-aggregate-test-proof-surface
```

The targeted warning grep is expected to return no dead-code warnings for
`registered_agent_workspace`, `registered_agent`, `hosted_identity_status`, or
the `profile` / `name` fields.

## Results

- `cargo test agent_aggregate --lib`: pass; 26 tests passed.
- `cargo check --features axon-pb --lib --all-targets 2>&1 | tee target/agent-aggregate-test-proof-surface-check.log`: pass.
- Targeted warning grep over `target/agent-aggregate-test-proof-surface-check.log`: no Agent aggregate warning matches.
- `bash tools/scripts/check-architecture-convergence.sh`: pass.
- `bash tests/scripts/test_check_architecture_convergence.sh`: pass.
- `git diff --check -- src/daemon/persistence/agent_aggregate.rs tools/scripts/check-architecture-convergence.sh tests/scripts/test_check_architecture_convergence.sh pr/20260716-agent-aggregate-test-proof-surface`: pass.

The compile log still reports the pre-existing, unrelated
`cancel_invocations_for_handle` warning in `src/ffi/invocation/mod.rs`.
