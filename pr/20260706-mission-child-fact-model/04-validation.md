Validation plan:
- Go unit tests for strict child fact parsing and plan conformance.
- Python unit tests for strict child fact parsing and plan conformance.
- Shared conformance tests assert the new expectation and mismatch details.
- Run focused Go/Python Mission and conformance tests.
- Run parity/scaffold checks and cargo conformance runner for Go/Python at minimum.

Results:
- PASS: `go test ./... -run 'Mission'` in `sdk/go`
- PASS: `PYTHONPATH=tests ./.venv/bin/python -m unittest tests.test_mission` in `sdk/python`
- PASS: `go test ./... -run 'Identity|Signing|Conformance'` in `sdk/go`
- PASS: `PYTHONPATH=tests ./.venv/bin/python -m unittest tests.test_identity tests.test_signing tests.test_conformance` in `sdk/python`
- PASS: `tools/scripts/check-sdk-parity-matrix.sh --self-test`
- PASS: `bash tools/scripts/check-sdk-scaffold.sh`
- PASS: `cargo run --bin sdk-conformance-runner -- --language go --adapter-report sdk/conformance/runner/go-action-adapter-report.json`
- PASS: `cargo run --bin sdk-conformance-runner -- --language python --adapter-report sdk/conformance/runner/python-action-adapter-report.json`
- PASS: `git diff --check`
