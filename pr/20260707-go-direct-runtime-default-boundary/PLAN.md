# Go Direct Runtime Default Boundary

## Intent

Keep product repositories free of generated Axon protobuf and raw gRPC transport
ownership by keeping the concrete daemon gRPC projection inside an SDK-owned
Runtime Core provider seam. Rust/C ABI remains the semantic SDK implementation
target; this Go provider is a bounded daemon-wire projection for the backend
cutover.

## Invariants

- Default `go test ./...` for `sdk/go` must stay protobuf-registry safe for
  consumers that still import their own generated `axon.v1` packages.
- Default Go SDK package sources must not import `internal/axonpb` or gRPC
  transport packages unless the source is an explicitly tagged SDK-owned direct
  runtime provider or the private generated adapter package.
- Public Runtime Core objects continue to expose generic `RuntimeConnector` and
  `RuntimeTransport` seams without Axon protobuf types in their signatures.
- Backend may depend on the SDK provider only after deleting backend-local
  generated protobufs or after the provider is isolated into a separate SDK
  package; backend must not keep generated protobufs or raw daemon transport
  packages at cutover.

## Boundary Proof

`DirectDaemonRuntimeTransport` is a provider-backed seam, not a second protocol
implementation. It serializes SDK DTOs to the daemon's current
`axon.v1.Invocation` endpoint when explicitly enabled, projects descriptor refs
to owner-local daemon ability names through SDK/Axon-delegated helpers, and
carries the original descriptor ref in daemon metadata for strict signature
admission.

## Verification

- `go test ./...` in `sdk/go`
- `go test -tags=easynet_direct_runtime ./...` in `sdk/go`
- Go SDK import-boundary test allows only the tagged direct provider and private
  generated adapter to import `internal/axonpb` or gRPC.
- Backend SDK-only boundary checker remains the acceptance gate for the larger
  backend cutover and is expected to fail until direct backend transport is
  removed.
