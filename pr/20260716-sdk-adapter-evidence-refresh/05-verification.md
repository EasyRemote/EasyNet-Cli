# Verification Plan

```bash
python3 sdk/conformance/refresh_adapter_report_evidence.py --write
python3 sdk/conformance/refresh_adapter_report_evidence.py --check
SDK_CONFORMANCE_RESULT_DIR=target/sdk-conformance-live-results.adapter-evidence \
  bash tools/scripts/check-sdk-conformance-reports.sh
EASYNET_SDK_PARITY_RESULTS_DIR=target/sdk-conformance-live-results.adapter-evidence \
EASYNET_SDK_PARITY_ALLOW_SNAPSHOT_RESULTS=1 \
  bash tools/scripts/check-sdk-parity-matrix.sh
bash tools/scripts/check-architecture-convergence.sh
git diff --check -- sdk/conformance/runner/c-abi-action-adapter-report.json sdk/conformance/runner/rust-action-adapter-report.json pr/20260716-sdk-adapter-evidence-refresh
```

## Results

- `python3 sdk/conformance/refresh_adapter_report_evidence.py --write`: pass; adapter evidence refreshed.
- `python3 sdk/conformance/refresh_adapter_report_evidence.py --check`: pass; evidence is current.
- `SDK_CONFORMANCE_RESULT_DIR=target/sdk-conformance-live-results.adapter-evidence bash tools/scripts/check-sdk-conformance-reports.sh`: pass.
- `EASYNET_SDK_PARITY_RESULTS_DIR=target/sdk-conformance-live-results.adapter-evidence EASYNET_SDK_PARITY_ALLOW_SNAPSHOT_RESULTS=1 bash tools/scripts/check-sdk-parity-matrix.sh`: pass.
- Focused live result directory contains `rust`, `c_abi`, `go`, `python`,
  `node`, `java`, `swift`, and `source-attestation` result files.
- `bash tools/scripts/check-architecture-convergence.sh`: pass.
