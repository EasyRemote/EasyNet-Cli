# Verification

## Commands

```text
tests/scripts/test_check_architecture_convergence.sh
tools/scripts/check-architecture-convergence.sh
cargo test --features axon-pb voice_capability_state_evidence_is_honest_about_provider_boundaries --lib
cargo test --features axon-pb explicit_voice_repository_registers_only_hub_call_aggregate_routes --lib
git diff --check
```

## Result

All commands passed.

## Notes

The architecture self-test now includes a positive fixture for the qualified
voice provider assembly boundary and a negative fixture that routes live voice
handlers through a raw repository backed by daemon-local state.
