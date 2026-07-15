Verification
============

Executed checks:

- `go test ./...` from `sdk/go`: OK.
- `PYTHONPATH=sdk/python:../EasyNet-Axon/sdk/python python3 -m pytest -q sdk/python/tests/test_access_control.py`: OK.
- `tools/scripts/check-python-sdk-static-contract.sh`: OK.
- `tools/scripts/check-sdk-canonical-public-api.sh`: OK.
- `tools/scripts/check-sdk-parity-matrix.sh --self-test`: OK.
- `tools/scripts/check-sdk-product-neutrality.sh`: OK.
- `tools/scripts/check-sdk-ura-naming.sh`: OK.
- `tools/scripts/check-architecture-convergence.sh`: OK.
- `git diff --check`: OK.

Notes:

- `python3.12 -m pytest` was not used because that interpreter lacks pytest in
  this workspace. The repository-provisioned `python3` pytest environment was
  used with the same `PYTHONPATH` shape as the SDK scripts.
- The canonical public API gate did not require public API regeneration.
