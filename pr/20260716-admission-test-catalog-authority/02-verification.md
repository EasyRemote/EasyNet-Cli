Verification plan

- `cargo test --features axon-pb principal_lifecycle_admission_denies_suspended_user_even_when_key_remains_trusted --lib -- --nocapture`
- `cargo test --features axon-pb principal_lifecycle_admission_denies_deleted_user_even_when_key_remains_trusted --lib -- --nocapture`
- `cargo test --features axon-pb principal_ --lib`
- `tools/scripts/check-architecture-convergence.sh`
- `git diff --check`

Results

- `cargo test --features axon-pb principal_lifecycle_admission_denies_suspended_user_even_when_key_remains_trusted --lib -- --nocapture`: PASS
- `cargo test --features axon-pb principal_lifecycle_admission_denies_deleted_user_even_when_key_remains_trusted --lib -- --nocapture`: PASS
- `cargo test --features axon-pb principal_ --lib`: PASS, 34 passed
- `tools/scripts/check-architecture-convergence.sh`: PASS
- `git diff --check`: PASS

Decision

- The helper keeps `build_system_registry()` because the tests need a mixed
  Hub/Device descriptor baseline.
- The helper no longer hot-registers `session.open`; admission only needs its
  descriptor binding, and the descriptor-only carrier already exists under the
  runtime-admin template authority.
