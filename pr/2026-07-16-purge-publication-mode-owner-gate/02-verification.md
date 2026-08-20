# Verification

## Commands

```text
bash -n tools/scripts/check-architecture-convergence.sh
tests/scripts/test_check_architecture_convergence.sh
tools/scripts/check-architecture-convergence.sh
cargo test --features axon-pb every_device_capable_mode_has_an_explicit_publication_recovery_contract --lib
git diff --check
```

## Result

All commands passed.

## Notes

The architecture self-test now includes a positive fixture for explicit purge publication recovery ownership and a negative fixture where `DaemonMode::Both` incorrectly borrows the upstream session owner and validates after purge recovery.
