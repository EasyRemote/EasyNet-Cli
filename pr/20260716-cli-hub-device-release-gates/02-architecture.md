# Architecture

## Boundary

This slice belongs to the EasyNet-Cli product runtime boundary. Axon owns
Invocation, receipt and protocol semantics; EasyNet-Cli owns daemon process
lifecycle, local key-service process wiring, CLI commands, and release package
shape.

## Layering

- `packaging/release/dev-install-local.sh` is a developer packaging adapter. It
  mirrors release installer artifact ownership and must not define runtime
  semantics.
- `tools/scripts/check-release-package-contract.sh` is the static source of
  truth for package-shape drift.
- `tools/scripts/check-sdk-cutover-readiness.sh` composes release/package gates
  with SDK and downstream gates; it should fail early on local artifact drift.
- `tools/scripts/cli-hub-device-daemon-e2e.sh` is the executable product-path
  proof. Its self-test is cheap gate coverage; the full run remains opt-in.

## Deletion/Convergence

The retired callback-socket product path is not reintroduced. The E2E harness
proves CLI hub/device behavior through daemon-owned commands and the embedded
local runtime rather than a backend or runtime-dispatch callback.
