# Verification

| Check | Result | Evidence |
| --- | --- | --- |
| Rust invocation history tests | Passed | `cargo test --lib daemon::ability::builtins::governance::invocation_history::tests -- --nocapture` — 31 passed |
| CLI invocation group tests | Passed | `cargo test --lib cli::commands::groups::invocation::tests -- --nocapture` — 10 passed |
| Node receipt filter tests | Passed | `npm test` in `sdk/node` — 46 passed |
| Go/Python receipt parity checks | Passed | `go test ./...` in `sdk/go`; `PYTHONPATH=sdk/python:../EasyNet-Axon/sdk/python python -m pytest sdk/python/tests/test_receipt.py sdk/python/tests/test_authorized_runtime_session.py` — 41 passed |
| SDK public API inventory | Passed | `tools/scripts/check-sdk-canonical-public-api.sh` — `canonical-public-api: OK` |
| `cargo fmt --check` | Passed | no rustfmt diff |
| `git diff --check` | Passed | no whitespace errors |
| canonical runtime convergence v2 gate | Passed | `tools/scripts/check-canonical-runtime-convergence-v2.sh` — `canonical-runtime-convergence-v2: OK` |
| architecture convergence gate | Passed | `tools/scripts/check-architecture-convergence.sh` — `architecture-convergence: OK` |
