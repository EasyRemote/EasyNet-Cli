Verification plan

- `python3 provider_routes/generate_principal_routes.py --check`
- `gofmt -w sdk/go/principal.go sdk/go/principal_routes_gen.go sdk/go/principal_test.go`
- `cargo fmt -- src/cli/commands/groups/mod.rs src/cli/commands/groups/principal.rs src/cli/commands/groups/principal_routes_gen.rs`
- `cargo test --features axon-pb cli::commands::groups::principal::tests:: --lib`
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
- `gofmt -w sdk/go/principal.go sdk/go/principal_routes_gen.go sdk/go/principal_test.go`: PASS
- `cargo fmt -- src/cli/commands/groups/mod.rs src/cli/commands/groups/principal.rs src/cli/commands/groups/principal_routes_gen.rs`: PASS
- `cargo test --features axon-pb cli::commands::groups::principal::tests:: --lib`: PASS, 18 passed
- `go test . -run 'TestRuntimePrincipalProvider|TestPrincipalLifecycleRoutesGeneratedFromManifest'`: PASS
- `PYTHONPATH=sdk/python python3 -m pytest -q sdk/python/tests/test_principal.py`: PASS, 9 passed
- `tools/scripts/check-python-sdk-static-contract.sh`: PASS
- `tools/scripts/check-sdk-canonical-public-api.sh`: PASS
- `tools/scripts/check-sdk-parity-matrix.sh --self-test`: PASS
- `tools/scripts/check-sdk-product-neutrality.sh`: PASS
- `tools/scripts/check-architecture-convergence.sh`: PASS
- `git diff --check`: PASS

Out-of-scope observation

- `cargo test --features axon-pb principal_ --lib` was too broad for this
  slice and surfaced two existing admission facade failures unrelated to
  Principal CLI route lowering: `session.open` resolves to multiple authority
  roots (`_system/device/session-open-template` and
  `easynet.run/device/ability-catalog-snapshot`). Treat this as a candidate
  follow-up root-fork slice for descriptor authority ownership.
