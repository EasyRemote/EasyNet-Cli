# Verification

## Planned Gates

- `python3 provider_routes/generate_runtime_admin_routes.py --check`
- `python3 provider_routes/generate_principal_routes.py --check`
- `python3 provider_routes/generate_access_control_routes.py --check`
- `python3 provider_routes/generate_receipt_routes.py --check`
- `go test . -run 'TestRuntimeAdmin|TestRuntimeAdminRoutesGeneratedFromManifest'`
- `PYTHONPATH=sdk/python python3 -m pytest -q sdk/python/tests/test_runtime_admin.py`
- `cargo test --features axon-pb daemon::ability::conformance --lib`
- `cargo test --features axon-pb runtime_admin_routes_are_generated_from_manifest --lib`
- `tools/scripts/check-sdk-canonical-public-api.sh`
- `tools/scripts/check-sdk-parity-matrix.sh --self-test`
- `tools/scripts/check-sdk-product-neutrality.sh`
- `tools/scripts/check-python-sdk-static-contract.sh`
- `tools/scripts/check-architecture-convergence.sh`
- `git diff --check`
- `git diff --cached --check`

## Results

- PASS: `python3 provider_routes/generate_runtime_admin_routes.py --check`
- PASS: `python3 provider_routes/generate_principal_routes.py --check`
- PASS: `python3 provider_routes/generate_access_control_routes.py --check`
- PASS: `python3 provider_routes/generate_receipt_routes.py --check`
- PASS: `go test . -run 'TestRuntimeAdmin|TestRuntimeAdminRoutesGeneratedFromManifest'`
- PASS: `PYTHONPATH=sdk/python python3 -m pytest -q sdk/python/tests/test_runtime_admin.py`
- PASS: `cargo test --features axon-pb daemon::ability::conformance --lib`
- PASS: `cargo test --features axon-pb runtime_admin_routes_are_generated_from_manifest --lib`
- PASS: `tools/scripts/check-sdk-canonical-public-api.sh`
- PASS: `tools/scripts/check-sdk-parity-matrix.sh --self-test`
- PASS: `tools/scripts/check-sdk-product-neutrality.sh`
- PASS: `tools/scripts/check-python-sdk-static-contract.sh`
- PASS: `tools/scripts/check-architecture-convergence.sh`
- PASS: `git diff --check`
