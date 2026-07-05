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
- Go Runtime-backed device history rejects daemon output rows whose device
  subject does not match the requested `device_ura`; the SDK fails closed rather
  than filtering product data in the facade.

Additional Go Runtime checks:

- `go test -count=1 ./...` from `sdk/go`
- `go test -count=1 -tags easynet_cabi ./...` from `sdk/go`
