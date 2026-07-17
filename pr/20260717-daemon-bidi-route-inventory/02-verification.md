# Verification

Planned checks:

- `cargo fmt --all -- --check`
- `cargo test --lib daemon::ability::conformance::tests::daemon_invocation_route_tables_are_classified_by_dispatchers -- --nocapture`
- `bash tools/scripts/check-architecture-convergence.sh`
- `bash tests/scripts/test_check_architecture_convergence.sh`
- `PYTHON_BIN=/opt/anaconda3/bin/python bash tools/scripts/check-canonical-runtime-convergence-v2.sh`
- `PYTHON_BIN=/opt/anaconda3/bin/python bash tests/scripts/test_check_canonical_runtime_convergence_v2.sh`

