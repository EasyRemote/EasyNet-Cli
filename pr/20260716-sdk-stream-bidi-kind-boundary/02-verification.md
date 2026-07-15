# Verification

- `python -m pytest sdk/python/tests/test_stream.py sdk/python/tests/test_bidi.py sdk/python/tests/test_transport.py`
- `go test ./sdk/go`
- `bash tools/scripts/check-go-sdk-seam.sh`
- `bash tools/scripts/check-python-sdk-seam.sh`
- `tools/scripts/check-architecture-convergence.sh`
- `git diff --check`
- `git diff --cached --check`
