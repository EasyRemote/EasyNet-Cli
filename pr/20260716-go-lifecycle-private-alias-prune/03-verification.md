Verification
============

Executed checks:

- `rg -n "\b(newDaemonHandle|requireDaemonRuntimeReady|daemonRuntimeReady|validDaemonState|wrapDaemonTransportError)\b" sdk/go/daemon.go sdk/go/*_test.go` - no references
- `go test ./...` from `sdk/go` - passed
- `tools/scripts/check-sdk-canonical-public-api.sh` - passed
- `tools/scripts/check-sdk-parity-matrix.sh --self-test` - passed
- `tools/scripts/check-sdk-product-neutrality.sh` - passed
- `tools/scripts/check-sdk-ura-naming.sh` - passed
- `tools/scripts/check-architecture-convergence.sh` - passed
- `git diff --check` - passed

Public API result:

- No public API model regeneration was required; the removed functions were
  unexported and definition-only.
