# Verification

Planned checks:

- `tools/scripts/check-canonical-runtime-convergence-v2.sh`
- `tools/scripts/check-canonical-runtime-convergence-v2.sh --self-test`
- `tools/scripts/check-architecture-convergence.sh`
- `cargo test -q runtime_descriptor_resolver --features axon-pb`
- `cargo test -q descriptor_catalog --features axon-pb`
- `cargo fmt --check`
- `git diff --check`

Results:

- `tools/scripts/check-canonical-runtime-convergence-v2.sh` — passed.
- `tools/scripts/check-canonical-runtime-convergence-v2.sh --self-test` —
  passed. The self-test now includes a fixture that otherwise follows the typed
  descriptor resolver shape but still defines `runtime_meta_descriptor_catalog_entries`.
- `tools/scripts/check-architecture-convergence.sh` — passed.
- `cargo test -q runtime_descriptor_resolver --features axon-pb` — passed
  (`6 passed` for the matching unit filter).
- `cargo test -q descriptor_catalog --features axon-pb` — passed (`4 passed`
  for the matching unit filter).
- `cargo fmt --check` — passed.
- `git diff --check` — passed.
- `codegraph sync && codegraph status` — passed; index is up to date.

Observed non-blocking debt:

- `tests/scripts/test_check_architecture_convergence.sh` currently fails in
  its own "canonical fixture should pass" setup on pre-existing rules outside
  this slice (`R25B`, `R90`, `R91`, `R92`, `R93`, `R99`). The real checkout
  passes `tools/scripts/check-architecture-convergence.sh`; the script fixture
  needs a separate refresh against the expanded gate set.
