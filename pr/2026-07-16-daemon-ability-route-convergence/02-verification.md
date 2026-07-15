# Verification

Planned checks for the executable architecture gate:

- `tests/scripts/test_check_architecture_convergence.sh`
- `tools/scripts/check-architecture-convergence.sh`
- `cargo test --features axon-pb daemon_invocation_route_tables_are_classified_by_dispatchers --lib`
- `git diff --check`

Completed checks:

- PASS: `tests/scripts/test_check_architecture_convergence.sh`
- PASS: `tools/scripts/check-architecture-convergence.sh`
- PASS: `cargo test --features axon-pb daemon_invocation_route_tables_are_classified_by_dispatchers --lib`
- PASS: `git diff --check`
