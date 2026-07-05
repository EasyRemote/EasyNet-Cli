# Verification

Completed checks:

- `cargo fmt`
- `cargo fmt --check`
- `cargo test events_`
- `PYTHONPATH=sdk/python:sdk/python/tests python -m pytest sdk/python/tests/test_events.py sdk/python/tests/test_cabi.py -q`
- `PYTHONPATH=sdk/python:sdk/python/tests python -m pytest sdk/python/tests/test_conformance.py sdk/python/tests/test_events.py sdk/python/tests/test_cabi.py -q`
- `PYTHONPATH=sdk/python:sdk/python/tests python -m pytest sdk/python/tests -q`
- `ruff check sdk/python`
- `bash tools/scripts/check-sdk-scaffold.sh`
- `gofmt -w sdk/go/cabi_events.go sdk/go/cabi_events_test.go`
- `(cd sdk/go && go test ./...)`
- JSON validation for new Events conformance fixtures and backend route coverage.

Failure-path coverage:

- Device cursor used for invocation stream.
- Missing invocation id.
- Device history limit bounds.
- Invalid event history projection row shape.
