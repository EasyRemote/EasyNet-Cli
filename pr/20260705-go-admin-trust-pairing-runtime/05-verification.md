# Verification

Completed checks:

- `gofmt -w admin_runtime.go admin_runtime_test.go` from `sdk/go`
- `go test -count=1 -run 'TestAdminRuntimeTransport'` from `sdk/go`
- `go test -count=1 ./...` from `sdk/go`
- `go test -count=1 -tags easynet_cabi ./...` from `sdk/go`
- `bash tools/scripts/check-sdk-scaffold.sh`

Covered behavior:

- Hub join/leave lower to complete Runtime Core Invocations with descriptor refs
  delegated through `IdentityClient`.
- Pairing preflight/create/validate and credential verification execute through
  `RuntimeClient.Invoke`.
- Pairing and credential projections fail through the existing typed DTO
  validators when daemon output omits required token, credential, scope,
  device, hub, expiry, or verification facts.
