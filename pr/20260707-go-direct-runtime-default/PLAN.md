# Go Direct Runtime Provider Boundary

## Objective

Promote the Go Daemon SDK direct daemon runtime transport from a tagged
experiment to an SDK-owned concrete Runtime Core provider that backend product
code can consume while deleting its own raw daemon gRPC and generated
`axon.v1` ownership. This preserves the daemon SDK requirements target graph:

```text
Product consumers -> EasyNet-Cli SDK Runtime Core -> easynet-daemon -> Axon adapter
```

The SDK owns the local daemon transport object model through generic runtime
interfaces. Product code must not keep raw daemon gRPC or generated `axon.v1`
protocol ownership. The only default Go code allowed to compile the daemon
gRPC/Axon protobuf projection is the SDK-owned direct runtime provider.

## Boundary Invariants

1. Axon remains the protocol source of truth. The direct provider may project
   SDK DTOs to generated daemon wire types, but it must not define new
   protocol truth.
2. Direct daemon runtime is a Runtime Core provider seam with OOP lifecycle
   ownership. It may dial UDS, serialize shared SDK DTOs, map transport errors,
   and project stream/bidi lifecycle events.
3. The transport must not implement new URA, canonical invocation, admission,
   receipt verification, or daemon policy semantics. Descriptor refs and URA
   ownership checks must go through SDK/Axon-delegated helpers.
4. Invocation defaults such as nonce generation may be exposed as explicit
   inspectable helpers; builders must still require complete tuple fields.

## Implementation

- Remove the `easynet_direct_runtime` build tag from `sdk/go/direct_runtime.go`
  and `sdk/go/direct_runtime_test.go` so backend can consume the provider
  without a product-specific build tag.
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

- Switch EasyNet backend from `internal/daemon_grpc` to the public Go SDK direct
  Runtime Core provider wrapped by `RuntimeSigningTransport` where caller
  signing is required.
- Delete backend generated `internal/pb/axon/v1` ownership after all product
  routes consume SDK-owned profiles.
- Align Python concrete daemon transport with Go after backend cutover.
