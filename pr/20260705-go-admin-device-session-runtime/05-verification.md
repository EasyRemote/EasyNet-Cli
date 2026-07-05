# Verification

Completed checks:

- `gofmt -w admin_runtime.go admin_runtime_test.go` from `sdk/go`
- `go test -count=1 -run 'TestAdminRuntimeTransport'` from `sdk/go`
- `go test -count=1 ./...` from `sdk/go`
- `go test -count=1 -tags easynet_cabi ./...` from `sdk/go`
- `bash tools/scripts/check-sdk-scaffold.sh`

Covered behavior:

- `AdminRuntimeTransport.CreateDeviceSession` delegates descriptor-ref
  construction to `IdentityClient` and executes through `RuntimeClient.Invoke`.
- `AdminRuntimeTransport.DeleteDeviceSession` executes through Runtime Core and
  preserves daemon-returned `device_ura` in the SDK result projection.
- Session create/delete args preserve daemon device-session identifiers and do
  not accept browser/product session ids.
