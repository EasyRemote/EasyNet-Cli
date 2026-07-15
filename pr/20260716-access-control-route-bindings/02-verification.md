Verification plan

- `python3 provider_routes/generate_access_control_routes.py --check`
- `gofmt -w sdk/go/access_control.go sdk/go/access_control_test.go sdk/go/access_control_routes_gen.go`
- `python3 -m compileall -q sdk/python/easynet_sdk/access_control.py sdk/python/easynet_sdk/_access_control_routes.py`
- `rustfmt --edition 2021 src/daemon/ability/access_control_routes_gen.rs src/daemon/ability/mod.rs src/daemon/ability/names/governance.rs src/daemon/ability/builtins/governance/access_control.rs`
- `go test . -run 'TestAccessControl|TestRuntimeAccessControl|TestAccessControlRoutesGeneratedFromManifest'`
- `PYTHONPATH=sdk/python python3 -m pytest -q sdk/python/tests/test_access_control.py`
- `cargo test --features axon-pb access_control --lib`
- `tools/scripts/check-sdk-canonical-public-api.sh`
- `tools/scripts/check-sdk-parity-matrix.sh --self-test`
- `tools/scripts/check-sdk-product-neutrality.sh`
- `tools/scripts/check-python-sdk-static-contract.sh`
- `tools/scripts/check-architecture-convergence.sh`
- `git diff --check`

Results

- PASS: `python3 provider_routes/generate_access_control_routes.py --check`
- PASS: `gofmt -w sdk/go/access_control.go sdk/go/access_control_test.go sdk/go/access_control_routes_gen.go`
- PASS: `python3 -m compileall -q sdk/python/easynet_sdk/access_control.py sdk/python/easynet_sdk/_access_control_routes.py`
- PASS: `rustfmt --edition 2021 src/daemon/ability/access_control_routes_gen.rs src/daemon/ability/mod.rs src/daemon/ability/names/governance.rs src/daemon/ability/builtins/governance/access_control.rs`
- PASS: `go test . -run 'TestAccessControl|TestRuntimeAccessControl|TestAccessControlRoutesGeneratedFromManifest'`
- PASS: `PYTHONPATH=sdk/python python3 -m pytest -q sdk/python/tests/test_access_control.py`
- PASS: `cargo test --features axon-pb access_control --lib`
- PASS: `tools/scripts/check-sdk-canonical-public-api.sh`
- PASS: `tools/scripts/check-sdk-parity-matrix.sh --self-test`
- PASS: `tools/scripts/check-sdk-product-neutrality.sh`
- PASS: `tools/scripts/check-python-sdk-static-contract.sh`
- PASS: `tools/scripts/check-architecture-convergence.sh`
- PASS: `git diff --check`
