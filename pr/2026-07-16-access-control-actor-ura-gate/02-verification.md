# Verification

## Commands

```text
tests/scripts/test_check_architecture_convergence.sh
tools/scripts/check-architecture-convergence.sh
cargo test --features axon-pb revoke_requires_a_canonical_actor_ura_before_opening_the_store --lib
git diff --check
```

## Result

All commands passed.

## Notes

The architecture self-test now includes a positive access-control revoke fixture and a negative scalar-fallback fixture where `actor_ura` is inferred from `owner_user_id`.
