# Verification

Executed checks:

- `go test -tags runtime_cabi . -run TestCABIRuntimeProviderProjectsDescriptorResolverLastError` from `sdk/go`
- `PYTHONPATH=sdk/python:sdk/python/tests:../EasyNet-Axon/sdk/python:$PYTHONPATH $SDK_CONFORMANCE_PYTHON -m unittest sdk/python/tests/test_cabi.py -k descriptor_resolution_projects_native_last_error`
- `bash tools/scripts/check-canonical-runtime-convergence-v2.sh --self-test`
- `bash tools/scripts/check-canonical-runtime-convergence-v2.sh`
- `cargo fmt --check`
- `cargo test canonical_runtime_convergence_v2_script_contract_holds --test script_checks`

Outcome: all passed. The SPEC v2 self-test rejects a temporary C ABI fixture
that drops typed descriptor last-error projection and preserves only generic or
legacy descriptor failure vocabulary.
