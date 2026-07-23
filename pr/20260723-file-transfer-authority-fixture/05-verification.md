# Verification

- `rg -n "AxonAbilityCatalog::new\\(|AxonAbilityCatalog::new_with_runtime\\(" src/daemon/ability/builtins/device_control/file_transfer.rs -S`
  - Passed with no matches.
- `cargo fmt --check`
  - Passed.
- `git diff --check`
  - Passed.
- `cargo test -q file_transfer::tests --features axon-pb`
  - Passed: 14 tests.
- `bash tools/scripts/check-architecture-convergence.sh`
  - Passed: `architecture-convergence: OK`.
- `bash tools/scripts/check-canonical-runtime-convergence-v2.sh`
  - Passed: `canonical-runtime-convergence-v2: OK`.
