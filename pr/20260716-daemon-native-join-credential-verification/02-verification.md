# Verification

## Passed Checks

- `cargo test -q load_and_verify_credentials_skips_backend_for_hub_ura_join_lineage --lib`
- `cargo test -q load_and_verify_credentials --lib`
- `tools/scripts/check-architecture-convergence.sh`
- `bash tests/scripts/test_check_architecture_convergence.sh`
- `rustfmt --edition 2024 --check src/cli/commands/start.rs`
- `git diff --check -- src/cli/commands/start.rs tools/scripts/check-architecture-convergence.sh tests/scripts/test_check_architecture_convergence.sh pr/20260716-daemon-native-join-credential-verification`
