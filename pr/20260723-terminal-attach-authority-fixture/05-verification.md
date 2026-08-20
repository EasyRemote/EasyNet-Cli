# Verification

- `rg -n "AxonAbilityCatalog::new\\(|AxonAbilityCatalog::new_with_runtime\\(" src/daemon/ability/builtins/device_control/terminal/attach.rs -S`
  - Passed with no matches.
- `cargo fmt --check`
  - Passed.
- `git diff --check`
  - Passed.
- `cargo test -q terminal::attach::tests --features axon-pb`
  - Passed: 8 tests.
- `bash tools/scripts/check-architecture-convergence.sh`
  - Passed: `architecture-convergence: OK`.
- `bash tools/scripts/check-canonical-runtime-convergence-v2.sh`
  - Passed: `canonical-runtime-convergence-v2: OK`.
