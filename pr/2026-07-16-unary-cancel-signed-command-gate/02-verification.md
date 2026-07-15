# Verification

## Commands

```text
bash -n tools/scripts/check-architecture-convergence.sh
tests/scripts/test_check_architecture_convergence.sh
tools/scripts/check-architecture-convergence.sh
cargo test --features axon-pb cancel_command_is_a_new_descriptor_bound_invocation --lib
cargo test --features axon-pb signed_invocation_cancel_command_replay_is_rejected --lib
git diff --check
```

## Result

All commands passed.

## Notes

The architecture self-test includes a negative fixture where
`request_cancel_signed` submits `signed.into_daemon_invocation()` directly and
where the cancel command omits the lifecycle-hash-bound independent draft.
