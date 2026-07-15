Verification plan

- `python3 provider_routes/generate_principal_routes.py --check`
- `rustfmt --edition 2021 src/daemon/ability/mod.rs src/daemon/ability/conformance.rs src/daemon/ability/principal_routes_gen.rs src/daemon/invocation/admission/principal_lifecycle.rs src/cli/commands/groups/principal_routes_gen.rs`
- `cargo test --features axon-pb principal_route_bindings_are_generated_from_manifest --lib`
- `cargo test --features axon-pb principal_ --lib`
- `cargo test --features axon-pb daemon::ability::conformance --lib`
- `go test . -run 'TestRuntimePrincipalProvider|TestPrincipalLifecycleRoutesGeneratedFromManifest'`
- `PYTHONPATH=sdk/python python3 -m pytest -q sdk/python/tests/test_principal.py`
- `tools/scripts/check-python-sdk-static-contract.sh`
- `tools/scripts/check-sdk-canonical-public-api.sh`
- `tools/scripts/check-sdk-parity-matrix.sh --self-test`
- `tools/scripts/check-sdk-product-neutrality.sh`
- `tools/scripts/check-architecture-convergence.sh`
- `git diff --check`

Results

- PASS: `rustfmt --edition 2021 src/daemon/ability/mod.rs src/daemon/ability/conformance.rs src/daemon/ability/principal_routes_gen.rs src/daemon/invocation/admission/principal_lifecycle.rs src/cli/commands/groups/principal_routes_gen.rs`
- PASS: `python3 provider_routes/generate_principal_routes.py --check`
- PASS: `cargo test --features axon-pb principal_route_bindings_are_generated_from_manifest --lib`
- PASS: `cargo test --features axon-pb principal_ --lib`
- PASS: `cargo test --features axon-pb daemon::ability::conformance --lib`
- PASS: `go test . -run 'TestRuntimePrincipalProvider|TestPrincipalLifecycleRoutesGeneratedFromManifest'`
- PASS: `PYTHONPATH=sdk/python python3 -m pytest -q sdk/python/tests/test_principal.py`
- PASS: `tools/scripts/check-sdk-canonical-public-api.sh`
- PASS: `tools/scripts/check-sdk-parity-matrix.sh --self-test`
- PASS: `tools/scripts/check-sdk-product-neutrality.sh`
- PASS: `tools/scripts/check-python-sdk-static-contract.sh`
- PASS: `tools/scripts/check-architecture-convergence.sh`
- PASS: `git diff --check`
