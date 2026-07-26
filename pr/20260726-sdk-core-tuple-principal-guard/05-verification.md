Verification
============

Planned checks
--------------

- `go test ./...` from `sdk/go`
- `sdk/python/.venv/bin/python -m pytest sdk/python/tests/test_invocation.py`
- `bash tools/scripts/check-architecture-convergence.sh`
- `bash tools/scripts/check-canonical-runtime-convergence-v2.sh`
- `cargo fmt --check`
- `git diff --check`

Results
-------

- `go test ./...` from `sdk/go`: passed.
- `sdk/python/.venv/bin/python -m pytest sdk/python/tests/test_invocation.py`: passed, 10 tests.
- `bash tools/scripts/check-architecture-convergence.sh`: passed.
- `bash tools/scripts/check-canonical-runtime-convergence-v2.sh`: passed.
- `cargo fmt --check`: passed.
- `git diff --check`: passed.

Gate-driven correction
----------------------

The first SPEC v2 run rejected a new `session/invocation_history`
placeholder in `sdk/go/invocation_test.go`. The fixture was changed to a
generic `runtime-state/read` all-zero principal so the SDK core tuple test
does not reintroduce retired receipt-history vocabulary outside the allowed
negative vectors.
