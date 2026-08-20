# API Contract

## Shell Environment Contract

`check-sdk-cutover-readiness.sh` may set:

- `SDK_CONFORMANCE_RESULT_DIR`: output directory for
  `check-sdk-conformance-reports.sh`.
- `EASYNET_SDK_PARITY_RESULTS_DIR`: input directory for
  `check-sdk-parity-matrix.sh`.
- `EASYNET_SDK_PARITY_ALLOW_SNAPSHOT_RESULTS=1`: allows parity validation of
  source snapshots emitted by the conformance report script.

## Compatibility

The public command remains:

```bash
bash tools/scripts/check-sdk-cutover-readiness.sh
```

Direct callers of `check-sdk-conformance-reports.sh` keep the existing default
result directory unless they explicitly set `SDK_CONFORMANCE_RESULT_DIR`.

## Error Contract

If conformance generation fails, the cutover script still reports the failing
gate and later gates may report missing artifacts in the run-scoped directory.
Those diagnostics must reference the current run, not stale default artifacts.
