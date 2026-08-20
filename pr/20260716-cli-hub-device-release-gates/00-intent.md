# Intent

## Goal

Close the release-gate gap for the unified CLI hub/device daemon path. The slice
binds three facts into normal verification: local development installs must
ship `easynet-keyring` with `easynet` and `easynet-daemon`; release package
contract checks must run as part of SDK cutover readiness; and the CLI-only
hub/device daemon E2E harness must at least self-test its command surface.

## Non-goals

- Do not run the full long-lived hub/device E2E by default.
- Do not introduce backend HTTP API dependencies into the CLI daemon proof.
- Do not change public CLI command names or invocation semantics.
- Do not touch unrelated Rust formatting churn.

## Acceptance Criteria

- `packaging/release/dev-install-local.sh` builds and installs `easynet-keyring`
  alongside the existing runtime binaries.
- `tools/scripts/check-sdk-cutover-readiness.sh --self-test` includes release
  package contract and CLI hub/device daemon E2E self-test gates.
- Normal cutover readiness includes the release package contract before live SDK
  smoke gates.
- Static release contract tests prove that removing keyring from the local
  developer installer fails the gate.
- The CLI-only E2E harness self-test proves its scoped command inventory remains
  present without starting daemons.
