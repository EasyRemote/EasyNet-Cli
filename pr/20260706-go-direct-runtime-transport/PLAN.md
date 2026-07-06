# Go Direct Runtime Transport

## Objective

Close the Go/Python Runtime Core transport gap without changing the daemon SDK
specification. Go must gain an SDK-internal direct daemon Invocation transport
over Axon gRPC UDS, matching the Python SDK's facade shape while keeping Axon
protobuf types private to the SDK implementation.

## Boundary Proof

- Axon remains the protocol source of truth. The Go transport uses generated
  Axon protobuf bindings only as a private SDK adapter.
- EasyNet-Cli SDK remains a facade over daemon Runtime Core. Public Go profile
  APIs continue to expose `RuntimeClient`, `InvocationDraft`, `StreamHandle`,
  and `BidiSession`, not Axon protobuf messages.
- Backend consumers must not import Axon or daemon internals; the existing
  backend SDK-only gate remains the product boundary.
- No fallback to legacy control-frame product invocation is introduced.

## Implementation Notes

1. Generate private Go bindings for the minimum Axon Invocation service protos
   under `sdk/go/internal/axonpb`.
2. Add a concrete `DirectDaemonRuntimeConnector` and
   `DirectDaemonRuntimeTransport` using gRPC over UDS.
3. Port the existing Python direct-runtime DTO projection semantics for unary,
   stream, and bidi paths.
4. Keep prepare/submit/handle observation delegated to an optional
   `RuntimeTransport`, matching Python's handle-transport composition.
5. Cover unary, stream, bidi, connector, error, and lifecycle behavior with Go
   tests using an in-process gRPC server over a Unix socket.

## Verification

- `go test ./...` in `sdk/go`
- `PYTHONPATH=sdk/python python -m pytest sdk/python/tests`
- `bash tools/scripts/check-sdk-parity-matrix.sh --self-test`
- `bash tools/scripts/check-sdk-scaffold.sh`
- `bash tools/scripts/check-easyremote-sdk-boundary.sh /Users/macbook.silan.tech/Documents/GitHub/EasyRemote`
