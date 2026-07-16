# Verification

## Focused Checks

- `rg -n 'RemoteAbilityInvocationTarget::for_target_owned_selector|remote_invoke::invoke_remote_target\\(' src/cli -S`
- `tools/scripts/check-architecture-convergence.sh`
- `bash tests/scripts/test_check_architecture_convergence.sh`

## Compile Checks

- `cargo test --features axon-pb exec --lib`
- `cargo test --features axon-pb device --lib`
- `cargo test --features axon-pb ability_catalog --lib`
- `cargo test --features axon-pb --test resolve_before_invoke_e2e`
