# Verification Plan

```bash
python3 sdk/conformance/refresh_adapter_report_evidence.py --write
python3 sdk/conformance/refresh_adapter_report_evidence.py --check
bash tools/scripts/check-sdk-conformance-reports.sh --self-test
SDK_CONFORMANCE_LANGUAGES=rust bash tools/scripts/check-sdk-conformance-reports.sh
bash tools/scripts/check-sdk-canonical-public-api.sh
```

# Results

- `python3 sdk/conformance/refresh_adapter_report_evidence.py --write`: refreshed the
  report evidence digest.
- `python3 sdk/conformance/refresh_adapter_report_evidence.py --check`: `adapter
  report evidence is current`.
- `python3 sdk/conformance/refresh_adapter_report_evidence.py --self-test`: `adapter
  report evidence refresh self-test ok`.
- `bash tools/scripts/check-sdk-conformance-reports.sh --self-test`:
  `check-sdk-conformance-reports self-test ok`.
- `SDK_CONFORMANCE_LANGUAGES=rust bash tools/scripts/check-sdk-conformance-reports.sh`:
  `check-sdk-conformance-reports ok`.
- `bash tools/scripts/check-sdk-canonical-public-api.sh`: `canonical-public-api: OK`.
- `git diff --check -- sdk/conformance/runner/rust-action-adapter-report.json
  pr/20260716-rust-adapter-evidence-refresh`: clean.
