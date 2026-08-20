Verification
============

Planned checks
--------------

- `go test ./sdk/go`
- `python -m pytest sdk/python/tests/test_errors.py`
- `bash tools/scripts/check-architecture-convergence.sh`
- `bash tools/scripts/check-canonical-runtime-convergence-v2.sh`
- `cargo fmt --check`
- `git diff --check`

Results
-------

- `go test ./...` from `sdk/go`: pass.
- `sdk/python/.venv/bin/python -m pytest sdk/python/tests/test_errors.py`: pass.
- `bash tools/scripts/check-architecture-convergence.sh`: pass.
- `bash tools/scripts/check-canonical-runtime-convergence-v2.sh`: pass.
- `cargo fmt --check`: pass.
- `git diff --check`: pass.
