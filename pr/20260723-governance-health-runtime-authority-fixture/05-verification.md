# Verification

- `cargo test -q governance::health::tests --features axon-pb`
  - Passed: 3 tests.
- `cargo fmt --check`
  - Passed.
- `git diff --check`
  - Passed.
- `bash tools/scripts/check-architecture-convergence.sh`
  - Passed: `architecture-convergence: OK`.
- `bash tools/scripts/check-canonical-runtime-convergence-v2.sh`
  - Passed: `canonical-runtime-convergence-v2: OK`.
- `/Users/macbook.silan.tech/.local/bin/codegraph sync`
  - Passed.
- `/Users/macbook.silan.tech/.local/bin/codegraph status`
  - Passed: index is up to date.
- `rg -n "AxonAbilityCatalog::new\\(|AxonAbilityCatalog::new_with_runtime\\(" src/daemon/ability/builtins/governance/health.rs -S`
  - Passed with no matches.
