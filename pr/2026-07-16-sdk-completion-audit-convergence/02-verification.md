# Verification

Planned:

- `bash tools/scripts/check-sdk-completion-audit.sh --self-test`
- `bash tools/scripts/check-sdk-completion-audit.sh --matrix-only`
- `bash tools/scripts/check-sdk-cutover-readiness.sh --self-test`
- `tests/scripts/test_check_sdk_scaffold.sh`
- `git diff --check`

Completed:

- PASS: `bash tools/scripts/check-sdk-completion-audit.sh --self-test`
- PASS: `bash tools/scripts/check-sdk-completion-audit.sh --matrix-only`
- PASS: `bash tools/scripts/check-sdk-cutover-readiness.sh --self-test`
- PASS: `bash tests/scripts/test_check_sdk_scaffold.sh`
- PASS: `bash -n tools/scripts/check-sdk-cutover-readiness.sh`
- PASS: `bash -n tools/scripts/check-sdk-completion-audit.sh`
- PASS: `git diff --check -- tools/scripts/check-sdk-completion-audit.sh tools/scripts/check-sdk-cutover-readiness.sh pr/2026-07-16-sdk-completion-audit-convergence/00-intent.md pr/2026-07-16-sdk-completion-audit-convergence/01-invariants.md pr/2026-07-16-sdk-completion-audit-convergence/02-verification.md`

Observed outside this slice:

- `bash tools/scripts/check-sdk-cutover-readiness.sh` reached
  `check-sdk-conformance-reports` and failed because the Go/Python
  `access_control/provider` adapter reports reference stale evidence hashes for
  `sdk/go/access_control_test.go` and `sdk/python/tests/test_access_control.py`.
  That is SDK/provider evidence drift, not a completion-audit state-machine
  failure, and should be handled as a separate convergence slice.
