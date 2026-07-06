# Verification

## 2026-07-07

- `go test .` in `sdk/go`: passed.
- `go test -tags easynet_direct_runtime . -run 'TestDirectDaemonRuntime' -count=1`
  in `sdk/go`: passed.
- `go test ./internal/svc -run TestSDKCompatibilityClientRetrieveAndDeleteFileUseDaemonSDKInvocation -count=1`
  in sibling `EasyNet/backend`: passed. This proves default SDK import no
  longer triggers the SDK internal Axon protobuf descriptor registration that
  collided with backend's temporary generated `internal/pb/axon/v1` package.
- `go test ./internal/svc -count=1` in sibling `EasyNet/backend`: passed.
- `bash tools/scripts/check-sdk-cutover-readiness.sh`: still fails only at the
  backend SDK-only boundary. Remaining failures are backend-owned raw Axon
  imports, `internal/daemon_grpc`, generated `internal/pb/axon/v1`, and
  `svc` imports of those packages.
