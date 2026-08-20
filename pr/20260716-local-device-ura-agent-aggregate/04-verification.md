# Verification

Planned checks:

- `rustfmt --edition 2021 --check src/daemon/identity/local_invocation.rs src/daemon/resources/context/clipboard_tracker.rs`
- `cargo test -q local_invocation --lib`
- `cargo test -q clipboard --lib`
- `bash tools/scripts/check-architecture-convergence.sh`
- `bash tests/scripts/test_check_architecture_convergence.sh`
- `git diff --check -- src/daemon/identity/local_invocation.rs src/daemon/resources/context/clipboard_tracker.rs tools/scripts/check-architecture-convergence.sh tests/scripts/test_check_architecture_convergence.sh pr/20260716-local-device-ura-agent-aggregate`

Results:

- `rustfmt --edition 2021 --check src/daemon/identity/local_invocation.rs src/daemon/resources/context/clipboard_tracker.rs`: passed.
- `cargo test -q local_invocation --lib`: passed, 1 test.
- `cargo test -q clipboard --lib`: passed, 6 tests.
- `bash tools/scripts/check-architecture-convergence.sh`: passed.
- `bash tests/scripts/test_check_architecture_convergence.sh`: passed.
- Scoped `git diff --check`: passed.
