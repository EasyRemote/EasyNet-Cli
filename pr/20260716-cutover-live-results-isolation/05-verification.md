# Verification Plan

```bash
bash -n tools/scripts/check-sdk-cutover-readiness.sh
bash tools/scripts/check-sdk-cutover-readiness.sh --self-test
bash tools/scripts/check-sdk-parity-matrix.sh --self-test
bash tools/scripts/check-sdk-conformance-reports.sh --self-test
git diff --check -- tools/scripts/check-sdk-cutover-readiness.sh pr/20260716-cutover-live-results-isolation
```

Full cutover readiness is intentionally broader than this slice and may take a
long time. The focused proof for this change is that the script self-test
rejects stale default live artifacts and validates only the run-scoped
directory.

# Results

- `bash -n tools/scripts/check-sdk-cutover-readiness.sh`: passed.
- `bash tools/scripts/check-sdk-cutover-readiness.sh --self-test`:
  `check-sdk-cutover-readiness self-test ok`.
- `bash tools/scripts/check-sdk-parity-matrix.sh --self-test`:
  `sdk parity matrix self-test ok`.
- `bash tools/scripts/check-sdk-conformance-reports.sh --self-test`:
  `check-sdk-conformance-reports self-test ok`.
