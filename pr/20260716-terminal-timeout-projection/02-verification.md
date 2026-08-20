# Verification

Passed:

```text
rustfmt --edition 2021 src/daemon/invocation/receipts/runtime_record.rs src/daemon/execution/loop_instance/mod.rs
cargo test --lib terminal_state_projects_every_axon_terminal_state -- --nocapture
cargo test --lib daemon::execution::loop_instance::tests -- --nocapture
bash tools/scripts/check-architecture-convergence.sh
bash tests/scripts/test_check_architecture_convergence.sh
git diff --check -- src/daemon/invocation/receipts/runtime_record.rs src/daemon/execution/loop_instance/mod.rs tools/scripts/check-architecture-convergence.sh tests/scripts/test_check_architecture_convergence.sh pr/20260716-terminal-timeout-projection
```

The R58 negative fixture proves the convergence gate rejects both a timeout
collapsed into `Failed` and a loop consumer that omits the distinct timeout
terminal branch.
