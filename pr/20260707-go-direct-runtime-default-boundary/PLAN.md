# Go Direct Runtime Default Boundary

## Intent

Keep the default Go Daemon SDK facade free of generated Axon protobuf and raw
gRPC transport ownership. A direct daemon runtime transport may exist only as
an explicitly enabled provider seam while Rust/C ABI remains the semantic SDK
implementation target.

## Invariants

- Default `go test ./...` for `sdk/go` must not compile `direct_runtime.go`.
- Default Go SDK package sources must not import `internal/axonpb` or gRPC
  transport packages unless the file is explicitly tagged as a direct runtime
  provider.
- Public Runtime Core objects continue to expose generic `RuntimeConnector` and
  `RuntimeTransport` seams without Axon protobuf types in their signatures.
- Backend must not depend on this direct provider as a product fallback.

## Boundary Proof

`DirectDaemonRuntimeTransport` is a provider-backed seam, not the canonical SDK
runtime implementation. It serializes DTOs to the daemon's current
`axon.v1.Invocation` endpoint for tagged builds only. Default SDK consumers see
only generic Runtime Core interfaces and C ABI-backed profile transports.

## Verification

- `go test ./...` in `sdk/go`
- `go test -tags=easynet_direct_runtime ./...` in `sdk/go`
- Go SDK import-boundary test rejects untagged `internal/axonpb` or gRPC
  transport dependencies.
- Backend SDK-only boundary checker remains the acceptance gate for the larger
  backend cutover and is expected to fail until direct backend transport is
  removed.
