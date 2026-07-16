# Verification

## Required Matrix

- Rust formatting and compilation.
- Targeted Rust tests for changed daemon/runtime modules.
- SDK conformance policy scripts.
- Go SDK unit tests covering invocation, receipt, lifecycle, and matrix
  evidence.
- Python SDK compile/tests covering invocation, receipt, lifecycle, and matrix
  evidence.
- Canonical runtime convergence V2 gate.
- URA terminology and schema derivation gates.

## Current Iteration Evidence

- `cargo fmt --check`
- `cargo check --lib --bins`
- `cargo test --lib --no-run`
- `cargo test --test script_checks`
- `bash tools/scripts/check-canonical-runtime-convergence-v2.sh`
- `bash tools/scripts/check-canonical-runtime-convergence-v2.sh --self-test`
- `bash tools/scripts/check-sdk-canonical-public-api.sh`
- `bash tools/scripts/check-sdk-product-neutrality.sh --self-test`
- `bash tools/scripts/check-sdk-product-neutrality.sh`
- `bash tests/scripts/test_check_canonical_runtime_convergence_v2.sh`
- `bash tools/scripts/check-skill-list-managed-dir-boundary.sh`
- `bash tests/scripts/test_check_skill_list_managed_dir_boundary.sh`
- `python3 sdk/conformance/refresh_adapter_report_evidence.py --check`
- `SDK_CONFORMANCE_RESULT_DIR=target/sdk-conformance-live-results.current bash tools/scripts/check-sdk-conformance-reports.sh`
- `EASYNET_SDK_PARITY_RESULTS_DIR=target/sdk-conformance-live-results.current EASYNET_SDK_PARITY_ALLOW_SNAPSHOT_RESULTS=1 bash tools/scripts/check-sdk-parity-matrix.sh`
- `cd sdk/go && go test ./...`
- `cd sdk/python && python3 -m py_compile $(find easynet_sdk -name '*.py' | sort)`

Result: passed.
