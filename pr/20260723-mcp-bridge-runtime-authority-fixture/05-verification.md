# Verification

- `rg -n "AxonAbilityCatalog::new\\(|AxonAbilityCatalog::new_with_runtime\\(" src/daemon/ability/builtins/integrations/mcp/bridge.rs -S`
  - Passed with no matches.
- `cargo fmt --check`
  - Passed.
- `git diff --check`
  - Passed.
- `cargo test -q integrations::mcp::bridge::tests --features axon-pb`
  - Passed: 17 tests.
- `bash tools/scripts/check-architecture-convergence.sh`
  - Passed: `architecture-convergence: OK`.
- `bash tools/scripts/check-canonical-runtime-convergence-v2.sh`
  - Passed: `canonical-runtime-convergence-v2: OK`.
- `/Users/macbook.silan.tech/.local/bin/codegraph sync`
  - Passed.
- `/Users/macbook.silan.tech/.local/bin/codegraph status`
  - Passed: index is up to date.
- `rg -n "AxonAbilityCatalog::new\\(|AxonAbilityCatalog::new_with_runtime\\(" src/daemon/ability/builtins/integrations -S`
  - Passed with no matches.
