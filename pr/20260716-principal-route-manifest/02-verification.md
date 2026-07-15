Verification plan

- `python3 provider_routes/generate_principal_routes.py --check`
- `go test . -run 'TestRuntimePrincipalProvider|TestPrincipalLifecycleRoutesGeneratedFromManifest'`
- `PYTHONPATH=sdk/python python3 -m pytest -q sdk/python/tests/test_principal.py`
- `tools/scripts/check-python-sdk-static-contract.sh`
- `tools/scripts/check-sdk-canonical-public-api.sh`
- `tools/scripts/check-sdk-parity-matrix.sh --self-test`
- `tools/scripts/check-sdk-product-neutrality.sh`
- `tools/scripts/check-architecture-convergence.sh`
- `git diff --check`

Results

- `python3 provider_routes/generate_principal_routes.py --check`: PASS
- `go test . -run 'TestRuntimePrincipalProvider|TestPrincipalLifecycleRoutesGeneratedFromManifest'`: PASS
- `PYTHONPATH=sdk/python python3 -m pytest -q sdk/python/tests/test_principal.py`: PASS, 9 passed
- `tools/scripts/check-python-sdk-static-contract.sh`: PASS
- `tools/scripts/check-sdk-canonical-public-api.sh`: PASS
- `tools/scripts/check-sdk-parity-matrix.sh --self-test`: PASS
- `tools/scripts/check-sdk-product-neutrality.sh`: PASS
- `tools/scripts/check-architecture-convergence.sh`: PASS
- `git diff --check`: PASS
