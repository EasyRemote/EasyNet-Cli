Verification plan

- `python3 provider_routes/generate_principal_routes.py --check`
- `python3 provider_routes/generate_access_control_routes.py --check`
- `python3 provider_routes/generate_receipt_routes.py --check`
- `gofmt -w sdk/go/receipt.go sdk/go/receipt_routes_gen.go sdk/go/receipt_test.go`
- `python3 -m compileall -q provider_routes/route_generator.py provider_routes/generate_principal_routes.py provider_routes/generate_access_control_routes.py provider_routes/generate_receipt_routes.py sdk/python/easynet_sdk/receipt.py sdk/python/easynet_sdk/_receipt_routes.py`
- `rustfmt --edition 2021 src/daemon/ability/receipt_routes_gen.rs src/daemon/ability/mod.rs src/daemon/ability/names/governance.rs src/daemon/ability/builtins/governance/invocation_history.rs`
- `go test . -run 'TestRuntimeReceiptProvider|TestReceiptRoutesGeneratedFromManifest|TestPrincipalLifecycleRoutesGeneratedFromManifest|TestAccessControlRoutesGeneratedFromManifest'`
- `PYTHONPATH=sdk/python python3 -m pytest -q sdk/python/tests/test_receipt.py sdk/python/tests/test_principal.py sdk/python/tests/test_access_control.py`
- `cargo test --features axon-pb invocation_history --lib`
- `cargo test --features axon-pb receipt_routes_are_generated_from_manifest --lib`
- `tools/scripts/check-sdk-canonical-public-api.sh`
- `tools/scripts/check-sdk-parity-matrix.sh --self-test`
- `tools/scripts/check-sdk-product-neutrality.sh`
- `tools/scripts/check-python-sdk-static-contract.sh`
- `tools/scripts/check-architecture-convergence.sh`
- `git diff --check`

Results

- PASS: `python3 provider_routes/generate_principal_routes.py --check`
- PASS: `python3 provider_routes/generate_access_control_routes.py --check`
- PASS: `python3 provider_routes/generate_receipt_routes.py --check`
- PASS: `gofmt -w sdk/go/receipt.go sdk/go/receipt_routes_gen.go sdk/go/receipt_test.go`
- PASS: `python3 -m compileall -q provider_routes/route_generator.py provider_routes/generate_principal_routes.py provider_routes/generate_access_control_routes.py provider_routes/generate_receipt_routes.py sdk/python/easynet_sdk/receipt.py sdk/python/easynet_sdk/_receipt_routes.py`
- PASS: `rustfmt --edition 2021 src/daemon/ability/receipt_routes_gen.rs src/daemon/ability/mod.rs src/daemon/ability/names/governance.rs src/daemon/ability/builtins/governance/invocation_history.rs`
- PASS: `go test . -run 'TestRuntimeReceiptProvider|TestReceiptRoutesGeneratedFromManifest|TestPrincipalLifecycleRoutesGeneratedFromManifest|TestAccessControlRoutesGeneratedFromManifest'`
- PASS: `PYTHONPATH=sdk/python python3 -m pytest -q sdk/python/tests/test_receipt.py sdk/python/tests/test_principal.py sdk/python/tests/test_access_control.py`
- PASS: `cargo test --features axon-pb invocation_history --lib`
- PASS: `cargo test --features axon-pb receipt_routes_are_generated_from_manifest --lib`
- PASS: `tools/scripts/check-sdk-canonical-public-api.sh`
- PASS: `tools/scripts/check-sdk-parity-matrix.sh --self-test`
- PASS: `tools/scripts/check-sdk-product-neutrality.sh`
- PASS: `tools/scripts/check-python-sdk-static-contract.sh`
- PASS: `tools/scripts/check-architecture-convergence.sh`
- PASS: `git diff --check`
