# Verification

- `cargo test -q ffi::invocation --features axon-pb`
- `cargo test -q ffi::invocation::backpressure`
- `PYTHONPATH=sdk/python python -m pytest sdk/python/tests/test_stream.py sdk/python/tests/test_bidi.py sdk/python/tests/test_transport.py`
- `(cd sdk/go && go test ./...)`
- `bash tools/scripts/check-sdk-parity-matrix.sh --self-test`
- `bash tools/scripts/check-sdk-product-neutrality.sh`
- `tools/scripts/check-architecture-convergence.sh`
- `git diff --check`
- `git diff --cached --check`
