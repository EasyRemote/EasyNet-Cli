# Verification Plan

1. Tagged Go direct-runtime tests prove every handle lifecycle operation fails
   closed without a delegate and still delegates with one.
2. R61 rejects Go source that retains synthetic direct handles, local prepare,
   or local signed submission.
3. Rebuild canonical API/parity artifacts and run SDK and architecture gates.

Results on 2026-07-16:

- `go test -tags easynet_direct_runtime ./...` from `sdk/go`: passed.
- `PYTHONPATH=sdk/python python -m pytest sdk/python/tests -q`: 374 passed.
- `bash tools/scripts/check-architecture-convergence.sh`: passed.
- `bash tests/scripts/test_check_architecture_convergence.sh`: passed.
- `bash tools/scripts/check-sdk-canonical-public-api.sh --self-test` and the
  regular check: passed.
- `bash tools/scripts/check-sdk-product-neutrality.sh --self-test` and the
  regular check: passed.
