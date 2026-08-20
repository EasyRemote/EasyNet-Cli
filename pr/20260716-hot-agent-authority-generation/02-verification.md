# Verification

1. `cargo test --features axon-pb hot_agent_authority_generation_overflow_fails_closed --lib`
2. `cargo test --features axon-pb hot_agent_authority_incarnation_overflow_fails_closed --lib`
3. `cargo test --features axon-pb stale_hot_agent_enrollment_rollback_cannot_remove_reenrolled_incarnation --lib`
4. `bash tests/scripts/test_check_architecture_convergence.sh`
5. `tools/scripts/check-architecture-convergence.sh`
6. `git diff --check`
