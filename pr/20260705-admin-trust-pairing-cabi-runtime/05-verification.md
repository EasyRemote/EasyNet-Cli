# Verification

- `cargo test admin_gateway` - passed.
- `cargo fmt --check` - passed.
- `go test -count=1 ./...` from `sdk/go` - passed.
- `go test -count=1 -tags easynet_cabi ./...` from `sdk/go` - passed.
- `PYTHONPATH=sdk/python:sdk/python/tests python -m pytest sdk/python/tests/test_cabi.py -q` - passed, 59 tests.
- `ruff check sdk/python` - passed.
- `bash tools/scripts/check-sdk-scaffold.sh` - passed.
- `git diff --check` - passed.
