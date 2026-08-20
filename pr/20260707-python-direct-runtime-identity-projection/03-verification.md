# Verification

Executed gates:

- `python -m py_compile sdk/python/easynet_sdk/direct_runtime.py sdk/python/easynet_sdk/transport.py sdk/python/easynet_sdk/environment.py sdk/python/tests/test_direct_runtime.py`
- `cd sdk/python && PYTHONPATH=. uv run pytest -q tests/test_direct_runtime.py`
- `cd sdk/python && PYTHONPATH=. uv run pytest -q`
- `git diff --check`
- `bash tools/scripts/check-sdk-cutover-readiness.sh`

Evidence:

- `tests/test_direct_runtime.py` proves unary, stream, and bidi emit
  identity-projected public ability names and typed Axon ability targets.
- Missing direct-runtime identity fails with `INVALID_ARGUMENT` before opening a
  daemon channel.
- Descriptor ownership mismatch fails before a daemon gRPC request is recorded.
- Connector-owned identity lifecycle closes exactly once.
- Full SDK readiness, EasyRemote product tests, backend product tests, and
  Python/Go live smoke gates passed.
