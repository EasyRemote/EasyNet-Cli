# Verification

Passed:

```text
rustfmt --edition 2021 src/daemon/invocation/dispatch/local_runtime_invoker.rs
cargo test --lib daemon::invocation::dispatch::local_runtime_invoker::tests -- --nocapture
cargo test -p easynet local_rpc_projects_finalized_output
bash tools/scripts/check-architecture-convergence.sh
bash tests/scripts/test_check_architecture_convergence.sh
git diff --check -- src/daemon/invocation/dispatch/local_runtime_invoker.rs tools/scripts/check-architecture-convergence.sh tests/scripts/test_check_architecture_convergence.sh pr/20260716-local-runtime-terminal-projection
```

R57 rejects direct local RPC terminal inference through `wait()` or event
snapshots. The adapter now calls `finalized()` and preserves exact Axon terminal
states for its public error projection.
