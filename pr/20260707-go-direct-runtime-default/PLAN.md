# Go Direct Runtime Provider Boundary

## Objective

Keep the Go Daemon SDK direct daemon runtime transport as an SDK-owned concrete
Runtime Core provider seam while preserving root-package protobuf registry
safety for backend code that has not yet deleted its generated `axon.v1`
ownership. This preserves the daemon SDK requirements target graph:

```text
Product consumers -> EasyNet-Cli SDK Runtime Core -> easynet-daemon -> Axon adapter
```

The SDK owns the local daemon transport object model through generic runtime
interfaces. Product code must not keep raw daemon gRPC or generated `axon.v1`
protocol ownership. The root SDK facade must not force all consumers to compile
the daemon gRPC/Axon protobuf projection while external product repositories
still carry generated `axon.v1` packages.

## Boundary Invariants

1. Axon remains the protocol source of truth. The direct provider may project
   SDK DTOs to generated daemon wire types, but it must not define new
   protocol truth.
2. Direct daemon runtime is an explicit Runtime Core provider seam with OOP
   lifecycle ownership. It may dial UDS, serialize shared SDK DTOs, map
   transport errors, and project stream/bidi lifecycle events.
3. The transport must not implement new URA, canonical invocation, admission,
   receipt verification, or daemon policy semantics. Descriptor refs and URA
   ownership checks must go through SDK/Axon-delegated helpers.
4. Invocation defaults such as nonce generation may be exposed as explicit
   inspectable helpers; builders must still require complete tuple fields.

## Implementation

- Keep the `easynet_direct_runtime` build tag on `sdk/go/direct_runtime.go`
  and `sdk/go/direct_runtime_test.go` until backend generated protobuf ownership
  is deleted or the provider is split into an isolated SDK subpackage.
- Keep generated Axon protobuf use behind `sdk/go/internal/axonpb` and allow it
  only from the SDK-owned direct runtime provider.
- Project `descriptor_ref` to owner-local daemon ability names using
  SDK/Axon-delegated helpers and carry the original signed descriptor ref in
  daemon metadata.
- Add cross-language nonce helpers for Go and Python Runtime Core Invocation
  builders without hiding the filled nonce from callers.
- Add `RuntimeSigningTransport` and `Signer.SignInvocationDraft` so product
  callers can wrap SDK-owned runtime transports with caller signing without
  copying backend-local `signEnvelope` / proto mapping logic.
- Preserve already-signed browser/user drafts. The signing decorator only signs
  drafts without `caller_signature`.

## Verification

- `go test ./...` under `sdk/go`.
- `PYTHONPATH=sdk/python python -m pytest sdk/python/tests/test_invocation.py`.
- `bash tools/scripts/check-sdk-cutover-readiness.sh`.

## Remaining Work

- Switch EasyNet backend from `internal/daemon_grpc` to a public Go SDK direct
  Runtime Core provider boundary wrapped by `RuntimeSigningTransport` where
  caller signing is required.
- Delete backend generated `internal/pb/axon/v1` ownership after all product
  routes consume SDK-owned profiles.
- Align Python concrete daemon transport with Go after backend cutover.
