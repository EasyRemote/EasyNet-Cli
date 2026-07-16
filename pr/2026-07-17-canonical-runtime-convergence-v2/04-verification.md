# Verification Log

## 2026-07-17

- `cargo fmt --check`
- `cargo check --lib --bins`
- `cargo test --lib --no-run`
- `cargo test --tests --no-run`
- `bash tools/scripts/check-canonical-runtime-convergence-v2.sh`
- `bash tools/scripts/check-sdk-canonical-public-api.sh`
- `bash tests/scripts/test_check_canonical_runtime_convergence_v2.sh`
- `cargo test --test script_checks canonical_runtime_convergence_v2_script_contract_holds`

Result: all passed.
