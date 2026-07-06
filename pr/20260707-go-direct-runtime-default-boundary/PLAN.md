# Go Direct Runtime Default Boundary

## Intent

Keep product repositories free of generated Axon protobuf and raw gRPC transport
ownership by moving the concrete daemon gRPC projection into the Go Daemon SDK
as a single SDK-owned Runtime Core provider. Rust/C ABI remains the semantic SDK
implementation target; this Go provider is a bounded daemon-wire projection for
the backend cutover.

## Invariants

- Default `go test ./...` for `sdk/go` compiles `direct_runtime.go` and proves
  the provider's unary, stream, bidi, and handle-delegation behavior.
- Default Go SDK package sources must not import `internal/axonpb` or gRPC
  transport packages except from the single SDK-owned direct runtime provider or
  private generated adapter package.
- Public Runtime Core objects continue to expose generic `RuntimeConnector` and
  `RuntimeTransport` seams without Axon protobuf types in their signatures.
- Backend may depend on this provider as its daemon SDK transport, replacing
  backend-local `internal/daemon_grpc`; backend must not import generated
  protobufs or raw daemon transport packages.

## Boundary Proof

`DirectDaemonRuntimeTransport` is a provider-backed seam, not a second protocol
implementation. It serializes SDK DTOs to the daemon's current
`axon.v1.Invocation` endpoint, projects descriptor refs to owner-local daemon
ability names through SDK/Axon-delegated helpers, and carries the original
descriptor ref in daemon metadata for strict signature admission.

## Verification

- `go test ./...` in `sdk/go`
- Go SDK import-boundary test allows only the concrete direct provider and
  private generated adapter to import `internal/axonpb` or gRPC.
- Backend SDK-only boundary checker remains the acceptance gate for the larger
  backend cutover and is expected to fail until direct backend transport is
  removed.
