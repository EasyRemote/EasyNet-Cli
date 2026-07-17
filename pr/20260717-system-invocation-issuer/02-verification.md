# Verification

Executed checks:

- `cargo fmt --all -- --check`
- `cargo test --lib daemon::axon_bridge::dispatch_shim::tests -- --nocapture`
- `bash tools/scripts/check-architecture-convergence.sh`
- `PYTHON_BIN=/opt/anaconda3/bin/python bash tools/scripts/check-canonical-runtime-convergence-v2.sh`
- `bash tests/scripts/test_check_architecture_convergence.sh`
- `PYTHON_BIN=/opt/anaconda3/bin/python bash tests/scripts/test_check_canonical_runtime_convergence_v2.sh`
