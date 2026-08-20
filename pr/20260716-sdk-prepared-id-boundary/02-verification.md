# Verification

- `PYTHONPATH=sdk/python python -m pytest sdk/python/tests/test_signing.py sdk/python/tests/test_cabi.py sdk/python/tests/test_runtime.py sdk/python/tests/test_ability_invocation.py`
- `go test ./...` from `sdk/go`
- `bash tools/scripts/check-sdk-canonical-public-api.sh`
- `bash tools/scripts/check-sdk-parity-matrix.sh --self-test`
- `bash tools/scripts/check-sdk-product-neutrality.sh`
- `bash tools/scripts/check-sdk-ura-naming.sh`
- `tools/scripts/check-architecture-convergence.sh`
- `git diff --check`
- `git diff --cached --check`

## 2026-07-16 Result

- `PYTHONPATH=sdk/python python -m pytest -q sdk/python/tests/test_signing.py sdk/python/tests/test_cabi.py sdk/python/tests/test_runtime.py sdk/python/tests/test_ability_invocation.py` -> 73 passed.
- `go test ./...` from `sdk/go` -> passed.
- `PYTHONPATH=sdk/python python -m pytest -q sdk/python/tests` -> 357 passed.
- `bash tools/scripts/check-sdk-canonical-public-api.sh` -> `canonical-public-api: OK`.
- `bash tools/scripts/check-sdk-parity-matrix.sh --self-test` -> `sdk parity matrix self-test ok`.
- `bash tools/scripts/check-sdk-product-neutrality.sh` -> `sdk-product-neutrality: OK`.
- `bash tools/scripts/check-sdk-ura-naming.sh` -> `SDK URA naming ok`.
- `tools/scripts/check-architecture-convergence.sh` -> `architecture-convergence: OK`.
- `git diff --check -- sdk/go/signing.go sdk/go/signing_test.go sdk/python/easynet_sdk/signing.py sdk/python/tests/test_signing.py pr/20260716-sdk-prepared-id-boundary` -> passed.
- `codegraph sync .` -> synced the 4 changed SDK files.
