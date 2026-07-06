# Go Direct Runtime Default Surface

## Objective

Move the Go Daemon SDK concrete direct runtime transport from an opt-in build
tag surface into the default SDK build. This advances the daemon SDK
requirements target graph:

```text
EasyNet backend -> EasyNet-Cli Go SDK -> easynet-daemon -> Axon adapter
```

The SDK owns the local daemon transport object model. Backend product code must
not keep raw daemon gRPC or generated `axon.v1` protocol ownership.

## Boundary Invariants

1. Axon remains the protocol source of truth. The Go SDK transport may use the
   SDK-internal generated Axon adapter, but product consumers must not import
   generated Axon protobuf packages.
2. Direct daemon runtime is a Runtime Core transport facade. It may dial UDS,
   serialize shared SDK DTOs, map transport errors, and project stream/bidi
   lifecycle events.
3. The transport must not implement new URA, canonical invocation, admission,
   receipt verification, or daemon policy semantics.
4. Invocation defaults such as nonce generation may be exposed as explicit
   inspectable helpers; builders must still require complete tuple fields.

## Implementation

- Remove the `easynet_direct_runtime` build tag from `sdk/go/direct_runtime.go`
  and `sdk/go/direct_runtime_test.go`.
- Keep generated Axon protobuf use behind `sdk/go/internal/axonpb`.
- Add cross-language nonce helpers for Go and Python Runtime Core Invocation
  builders without hiding the filled nonce from callers.

## Verification

- `go test ./...` under `sdk/go`.
- `PYTHONPATH=sdk/python python -m pytest sdk/python/tests/test_invocation.py`.
- `bash tools/scripts/check-sdk-cutover-readiness.sh`.

## Remaining Work

- Switch EasyNet backend from `internal/daemon_grpc` to the default Go SDK
  direct daemon runtime transport.
- Delete backend generated `internal/pb/axon/v1` ownership after all product
  routes consume SDK-owned profiles.
- Align Python concrete daemon transport with Go after backend cutover.
