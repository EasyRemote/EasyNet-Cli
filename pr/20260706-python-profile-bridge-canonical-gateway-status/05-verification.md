# Verification

Planned checks:

- `PYTHONPATH=tests uv run python -m unittest tests.test_profile_bridge tests.test_admin tests.test_conformance`
- `go test ./... -run 'AdminGateway|Conformance|ImportBoundary'` in `sdk/go`
- `tools/scripts/check-sdk-parity-matrix.sh --self-test`
- `bash tools/scripts/check-sdk-scaffold.sh`
- `cargo fmt --check`
- `cargo run --bin sdk-conformance-runner -- --language python --adapter-report sdk/conformance/runner/python-action-adapter-report.json`
- `cargo run --bin sdk-conformance-runner -- --language go --adapter-report sdk/conformance/runner/go-action-adapter-report.json`

Results:

- `PYTHONPATH=tests uv run python -m unittest tests.test_profile_bridge tests.test_admin tests.test_conformance`: passed, 40 tests.
- `python -m py_compile sdk/python/easynet_sdk/profile_bridge.py sdk/python/tests/test_profile_bridge.py sdk/python/tests/test_conformance.py`: passed.
- `gofmt -w conformance_test.go && go test ./... -run 'AdminGateway|Conformance|ImportBoundary'` in `sdk/go`: passed.
- `tools/scripts/check-sdk-parity-matrix.sh --self-test`: passed.
- `bash tools/scripts/check-sdk-scaffold.sh`: passed.
- `cargo fmt --check`: passed.
- `git diff --check`: passed.
- `cargo run --bin sdk-conformance-runner -- --language python --adapter-report sdk/conformance/runner/python-action-adapter-report.json`: passed.
- `cargo run --bin sdk-conformance-runner -- --language go --adapter-report sdk/conformance/runner/go-action-adapter-report.json`: passed.
- `cargo run --bin sdk-conformance-runner -- --language rust --adapter-report sdk/conformance/runner/rust-action-adapter-report.json`: passed.
- `cargo run --bin sdk-conformance-runner -- --language c_abi --adapter-report sdk/conformance/runner/c-abi-action-adapter-report.json`: passed.

Remaining aggregate cutover blocker:

- `tools/scripts/check-sdk-cutover-readiness.sh`: still fails because the sibling EasyNet backend contains raw Axon imports and direct `internal/daemon_grpc` transport packages/imports.
