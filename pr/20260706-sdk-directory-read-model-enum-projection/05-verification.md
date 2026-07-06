# Verification

Results:

- `go test ./...`: passed from `sdk/go`.
- `go test ./internal/axon`: passed from EasyNet backend.
- `bash tools/scripts/check-sdk-scaffold.sh`: passed.
- `bash tools/scripts/check-sdk-parity-matrix.sh --self-test`: passed.
- `git diff --check`: passed.

Remaining global gate:

- `bash tools/scripts/check-sdk-cutover-readiness.sh`: still fails at backend
  SDK-only boundary because EasyNet backend retains raw Axon imports,
  generated `internal/pb/axon/v1`, and `internal/daemon_grpc` transport
  packages outside this focused projection slice.
