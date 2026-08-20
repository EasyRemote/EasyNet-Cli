# Verification Plan

- `cargo test daemon::persistence::local_agents::tests --lib`
- targeted agent aggregate/lifecycle tests if affected by compile errors
- `cargo fmt --check`
- `bash tools/scripts/check-canonical-runtime-convergence-v2.sh --self-test`
- `bash tools/scripts/check-canonical-runtime-convergence-v2.sh`
- `bash tools/scripts/check-architecture-convergence.sh`
- `git diff --check`
