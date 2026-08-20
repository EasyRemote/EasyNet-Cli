Verification for profile error source-ref conformance:

- `PYTHONPATH=sdk/python python -m pytest sdk/python/tests/test_conformance.py -q`
- `PYTHONPATH=sdk/python python -m ruff check sdk/python/tests/test_conformance.py`
- `go test ./...` from `sdk/go`
- `bash tools/scripts/check-sdk-scaffold.sh`
- `git diff --check`
