# Verification

- `cargo test -q agents::lifecycle::tests::registration_makes_lifecycle_abilities_dispatchable --features axon-pb`
  - Passed: 1 test.
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
- `rg -n "AxonAbilityCatalog::new\\(|AxonAbilityCatalog::new_with_runtime\\(" src/daemon/ability/builtins/agents/lifecycle.rs -S`
  - Passed with no matches.
