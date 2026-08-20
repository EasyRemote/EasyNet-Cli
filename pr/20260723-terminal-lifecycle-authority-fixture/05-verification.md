# Verification

Passed:

- `rg -n "AxonAbilityCatalog::new\\(|AxonAbilityCatalog::new_with_runtime\\("
  src/daemon/ability/builtins/device_control/terminal/lifecycle.rs -S`
  - No output: terminal lifecycle tests no longer use ambient catalog
    constructors.
- `cargo fmt --check`
- `git diff --check`
- `cargo test -q terminal::lifecycle::tests --features axon-pb`
  - 21 passed.
- `bash tools/scripts/check-architecture-convergence.sh`
  - `architecture-convergence: OK`
- `bash tools/scripts/check-canonical-runtime-convergence-v2.sh`
  - `canonical-runtime-convergence-v2: OK`
- `/Users/macbook.silan.tech/.local/bin/codegraph sync`
  - Synced the changed Rust file.
- `/Users/macbook.silan.tech/.local/bin/codegraph status`
  - Index is up to date.
