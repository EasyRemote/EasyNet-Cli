# Verification

Executed gates:

- `cargo test --locked --lib registered_agent_runtime_projection_preserves_optional_forget_semantics -- --nocapture`
- `cargo test --locked --lib forget_with_hot_registrar_removes_the_discovery_only_learner_copy -- --nocapture`
- `cargo test --locked --lib forget_runtime_sync_unavailable_returns_error_and_keeps_tombstone -- --nocapture`
- `cargo test --locked --lib boot_sweep_converges_forget_tombstone_for_removed_learner -- --nocapture`
- `bash tests/scripts/test_check_architecture_convergence.sh`
- `bash tools/scripts/check-architecture-convergence.sh`
