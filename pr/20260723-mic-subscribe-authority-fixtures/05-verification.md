# Verification

Passed:

- `rg -n "AxonAbilityCatalog::new\\(|AxonAbilityCatalog::new_with_runtime\\("
  src/daemon/ability/builtins/resources/media/mic_subscribe.rs -S`
  - No output: `mic.subscribe` tests no longer use ambient catalog
    constructors.
- `cargo fmt --check`
- `git diff --check`
- `cargo test -q mic_subscribe::tests --features axon-pb`
  - 9 passed.
- `bash tools/scripts/check-architecture-convergence.sh`
  - `architecture-convergence: OK`
- `bash tools/scripts/check-canonical-runtime-convergence-v2.sh`
  - `canonical-runtime-convergence-v2: OK`
- `/Users/macbook.silan.tech/.local/bin/codegraph sync`
  - Synced the changed Rust file.
- `/Users/macbook.silan.tech/.local/bin/codegraph status`
  - Index is up to date.
