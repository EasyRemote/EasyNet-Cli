## Verification

Completed.

### Context

- Re-read the active goal objective and EasyNet engineering contract before the
  slice.
- Used `codegraph status` before selecting the root fork, then `codegraph sync .`
  after source changes appeared in the working tree.
- The selected root fork is wrapper lifecycle ownership: `run_bounded` could
  produce terminal statuses, but the report loop collapsed them into ordinary
  per-language failures and advanced to later languages.

### Checks

- `bash -n tools/scripts/check-sdk-conformance-reports.sh`
- `bash tools/scripts/check-sdk-conformance-reports.sh --self-test`
- `git diff --check -- tools/scripts/check-sdk-conformance-reports.sh pr/20260716-sdk-conformance-cancel-terminality`
- Short-timeout probe:
  `SDK_CONFORMANCE_LANGUAGES=rust,go SDK_CONFORMANCE_REPORT_TIMEOUT_SECONDS=1 bash tools/scripts/check-sdk-conformance-reports.sh`

The short-timeout probe exited with `124`, did not run `go`, and emitted no
cleanup noise after the cleanup trap was made best-effort.

### Wider Gate Note

A normal full conformance wrapper run is not claimed for this slice. During
exploration, once the old terminality bug let the wrapper advance past Rust, the
Go report path exposed unrelated evidence drift in `sdk/go/principal_test.go`
for `principal/lifecycle_seam`. Those source/report changes are outside this
terminality slice and were not staged here.
