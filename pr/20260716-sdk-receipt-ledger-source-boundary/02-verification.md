# Verification

- `go test ./...`
- `python -m pytest sdk/python/tests/test_receipt.py`
- `python3 sdk/conformance/rebuild_public_api_model.py --write`
- `bash tools/scripts/check-sdk-canonical-public-api.sh`
- `tools/scripts/check-architecture-convergence.sh`
- `git diff --check`
- `git diff --cached --check`
