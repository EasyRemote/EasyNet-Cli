# Verification

## Planned Checks

- `cargo test -q matches_self_target_ura --lib`
- `cargo test -q local_agent_target --lib`
- `cargo test -q local_target_projection --lib`
- `tools/scripts/check-architecture-convergence.sh`
- `bash tests/scripts/test_check_architecture_convergence.sh`
- `git diff --check -- src/daemon/invocation/admission/target_gate.rs tools/scripts/check-architecture-convergence.sh tests/scripts/test_check_architecture_convergence.sh pr/20260716-target-gate-agent-aggregate-boundary`

## Results

- `cargo test -q matches_self_target_ura --lib`: 2 passed.
- `cargo test -q local_agent_target --lib`: 2 passed.
- `cargo test -q local_target_projection --lib`: 1 passed.
- `tools/scripts/check-architecture-convergence.sh`: passed.
- `bash tests/scripts/test_check_architecture_convergence.sh`: passed.
- Scoped `git diff --check`: passed.
