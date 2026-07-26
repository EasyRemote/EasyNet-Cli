# Verification

Executed checks:

- `cargo test bootstrap_provider_ --lib` — passed, 1 test.
- `cargo test invoke_runtime_bootstrap_self_identity --lib` — passed, 2 tests.
- `cargo fmt --check` — passed.
- `git diff --check` — passed.
- `bash tools/scripts/check-architecture-convergence.sh` — passed.
- `bash tools/scripts/check-canonical-runtime-convergence-v2.sh` — passed.
