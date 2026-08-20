# Verification

Results:
- PASS: `go test ./... -run 'Signing|Identity|Conformance'` in `sdk/go`
- PASS: `PYTHONPATH=tests uv run python -m unittest tests.test_signing tests.test_identity tests.test_conformance` in `sdk/python`
- PASS: `tools/scripts/check-sdk-parity-matrix.sh --self-test`
- PASS: `bash tools/scripts/check-sdk-scaffold.sh`
- PASS: `cargo run --bin sdk-conformance-runner -- --language go --adapter-report sdk/conformance/runner/go-action-adapter-report.json`
- PASS: `cargo run --bin sdk-conformance-runner -- --language python --adapter-report sdk/conformance/runner/python-action-adapter-report.json`
- PASS: `cargo run --bin sdk-conformance-runner -- --language rust --adapter-report sdk/conformance/runner/rust-action-adapter-report.json`
- PASS: `cargo run --bin sdk-conformance-runner -- --language c_abi --adapter-report sdk/conformance/runner/c-abi-action-adapter-report.json`
- PASS: `cargo fmt --check`
- PASS: `git diff --check`
