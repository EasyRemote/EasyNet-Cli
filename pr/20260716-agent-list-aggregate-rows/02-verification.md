# Verification

Planned checks:

- `rustfmt --edition 2021 --check src/daemon/ability/builtins/agents/list.rs`
- `cargo test -q list_agents --lib`
- `bash tools/scripts/check-architecture-convergence.sh`
- `bash tests/scripts/test_check_architecture_convergence.sh`
- scoped `git diff --check`

Results:

- `rustfmt --edition 2021 --check src/daemon/ability/builtins/agents/list.rs`: passed.
- `cargo test -q list_agents --lib`: passed, 9 tests.
- `bash tools/scripts/check-architecture-convergence.sh`: passed.
- `bash tests/scripts/test_check_architecture_convergence.sh`: passed.
- `git diff --check -- src/daemon/ability/builtins/agents/list.rs tools/scripts/check-architecture-convergence.sh tests/scripts/test_check_architecture_convergence.sh pr/20260716-agent-list-aggregate-rows`: passed.
