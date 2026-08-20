Verification log

Executed in a temporary clean worktree based on commit `784ee347e` plus this
Go/Python runtime-identity slice:

- `go test ./...` in `sdk/go` - passed.
- `PYTHONPATH=sdk/python python -m pytest sdk/python/tests/test_runtime_identity.py sdk/python/tests/test_runtime_environment.py` - 11 passed.
- `tools/scripts/check-sdk-canonical-public-api.sh` - passed.
- `tools/scripts/check-sdk-parity-matrix.sh --self-test` - passed.
- `tools/scripts/check-sdk-product-neutrality.sh` - passed.
- `tools/scripts/check-sdk-ura-naming.sh` - passed.
- `tools/scripts/check-architecture-convergence.sh` - passed.
- `git diff --check` - passed.

`python3.12` was used for canonical public API regeneration to keep the
recorded Python AST parser stable with the existing model.

## 2026-07-16 Current Worktree Result

- `PYTHONPATH=sdk/python python -m pytest -q sdk/python/tests/test_runtime_identity.py` -> 7 passed.
- `go test -run 'TestRuntimeSigningIdentity|TestLoadRuntimeSigningIdentity|TestEnsureRuntimeSigningIdentity' ./...` from `sdk/go` -> passed.
- `PYTHONPATH=sdk/python python -m pytest -q sdk/python/tests` -> 359 passed.
- `go test ./...` from `sdk/go` -> passed.
- `bash tools/scripts/check-sdk-canonical-public-api.sh` -> `canonical-public-api: OK`.
- `bash tools/scripts/check-sdk-parity-matrix.sh --self-test` -> `sdk parity matrix self-test ok`.
- `bash tools/scripts/check-sdk-product-neutrality.sh` -> `sdk-product-neutrality: OK`.
- `bash tools/scripts/check-sdk-ura-naming.sh` -> `SDK URA naming ok`.
- `tools/scripts/check-architecture-convergence.sh` -> `architecture-convergence: OK`.
- `git diff --check -- sdk/go/runtime_identity.go sdk/go/runtime_identity_test.go sdk/python/easynet_sdk/runtime_identity.py sdk/python/easynet_sdk/__init__.py sdk/python/tests/test_runtime_identity.py sdk/conformance/canonical-public-api.json sdk/conformance/sdk-parity-matrix.json pr/20260716-sdk-runtime-identity-explicit-endpoint` -> passed.
- `codegraph sync .` -> synced 3 changed files.
