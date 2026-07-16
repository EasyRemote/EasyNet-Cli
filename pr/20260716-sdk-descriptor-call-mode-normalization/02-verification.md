# Verification

1. Python unit test records the request emitted for a whitespace-only mode and
   requires `call_mode == "rpc"`.
2. Architecture convergence rule R60 rejects a Python runtime facade that
   forwards an unnormalized `call_mode`.
3. Rebuild the canonical public API model and run both SDK and architecture
   conformance gates.

Results on 2026-07-16:

- `PYTHONPATH=sdk/python python -m pytest sdk/python/tests -q`: 374 passed.
- `go test ./...` from `sdk/go`: passed.
- `bash tools/scripts/check-architecture-convergence.sh`: passed.
- `bash tests/scripts/test_check_architecture_convergence.sh`: passed.
- `bash tools/scripts/check-sdk-canonical-public-api.sh --self-test` and the
  regular check: passed.
- `bash tools/scripts/check-sdk-product-neutrality.sh --self-test` and the
  regular check: passed.
