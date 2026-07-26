# Verification

## Commands

- `cd sdk/go && gofmt -w receipt.go receipt_test.go && go test ./...`
- `cd sdk/python && uv run pytest tests/test_receipt.py tests/test_authorized_runtime_session.py -q`
- `bash tools/scripts/check-architecture-convergence.sh`
- `bash tools/scripts/check-canonical-runtime-convergence-v2.sh`
- `git diff --check`
- `cargo fmt --check`

## Result

All commands passed.
