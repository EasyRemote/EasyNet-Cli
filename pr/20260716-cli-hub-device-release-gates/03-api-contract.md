# API Contract

## Script Interfaces

- `packaging/release/dev-install-local.sh [--debug] [--no-install]`
  - Builds `easynet`, `easynet-daemon`, `easynet-keyring`, and the native bridge.
  - Installs the three binaries to the configured install directory unless
    `--no-install` is set.
  - Fails if any required runtime artifact is missing.

- `tools/scripts/check-sdk-cutover-readiness.sh --self-test`
  - Runs cheap self-tests for all included gates.
  - Must include the release package contract and CLI hub/device daemon E2E
    self-test.

- `tools/scripts/check-sdk-cutover-readiness.sh`
  - Runs static and live cutover gates.
  - Must include release package contract before live SDK/product smokes.

- `tools/scripts/cli-hub-device-daemon-e2e.sh --self-test`
  - Does not start daemons or write product state.
  - Checks script syntax and required CLI command coverage markers.

## Error Contract

Missing `easynet-keyring` in any release/local install path is a hard failure.
The caller must install the complete runtime artifact set rather than falling
back to in-process key material or stale binaries.
