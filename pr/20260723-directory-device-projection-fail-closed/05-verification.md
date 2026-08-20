# Verification

All planned checks passed on 2026-07-23.

- `cargo test -q presence_adapter_tests --lib`
  - Result: passed, 7 tests.
- `cargo test -q apply_snapshot_rejects_invalid_agent_ura_without_mutating_view --lib`
  - Result: passed, 1 test.
- `cargo test -q apply_upsert_rejects_invalid_agent_ura_without_mutating_view --lib`
  - Result: passed, 1 test.
- `cargo test -q build_subscribe_directory_v2_snapshot --lib`
  - Result: passed, 2 tests.
- `cargo fmt --check`
  - Result: passed.
- `git diff --check`
  - Result: passed.
- `bash tools/scripts/check-architecture-convergence.sh`
  - Result: `architecture-convergence: OK`.
- `bash tools/scripts/check-canonical-runtime-convergence-v2.sh`
  - Result: `canonical-runtime-convergence-v2: OK`.
- `/Users/macbook.silan.tech/.local/bin/codegraph sync .`
  - Result: synced 3 changed source files.
- `/Users/macbook.silan.tech/.local/bin/codegraph query agent_ura_to_node_id`
  - Result: no results found.
- `/Users/macbook.silan.tech/.local/bin/codegraph query canonical_device_node_id --limit 20`
  - Result: validator indexed.

Observed but not counted as this change's verification:

- `cargo test -q directory --lib` also compiled this refactor, but the broad filter matched unrelated filesystem tests that require local joined credentials and failed in this machine state. Narrow focused tests above avoid that environmental dependency.
