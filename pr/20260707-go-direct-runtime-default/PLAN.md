# Go Direct Runtime Provider Boundary

## Objective

Keep the Go Daemon SDK default surface aligned to the generic Runtime Core while
retaining the concrete direct daemon runtime transport only as an explicit
provider-backed seam. This preserves the daemon SDK requirements target graph:

```text
Product consumers -> EasyNet-Cli SDK Runtime Core -> easynet-daemon -> Axon adapter
```

The SDK owns the local daemon transport object model through generic runtime
interfaces. Product code must not keep raw daemon gRPC or generated `axon.v1`
protocol ownership, and the default Go facade must not compile an Axon adapter.

## Boundary Invariants

1. Axon remains the protocol source of truth. The default Go SDK facade must not
   compile generated Axon protobuf packages or raw gRPC transport ownership.
2. Direct daemon runtime is an explicit Runtime Core provider seam. When enabled,
   it may dial UDS, serialize shared SDK DTOs, map transport errors, and project
   stream/bidi lifecycle events.
3. The transport must not implement new URA, canonical invocation, admission,
   receipt verification, or daemon policy semantics.
4. Invocation defaults such as nonce generation may be exposed as explicit
   inspectable helpers; builders must still require complete tuple fields.

## Implementation

- Keep the `easynet_direct_runtime` build tag on `sdk/go/direct_runtime.go`
  and `sdk/go/direct_runtime_test.go`.
- Keep generated Axon protobuf use behind `sdk/go/internal/axonpb`.
- Add cross-language nonce helpers for Go and Python Runtime Core Invocation
  builders without hiding the filled nonce from callers.
- Add `RuntimeSigningTransport` and `Signer.SignInvocationDraft` so product
  callers can wrap SDK-owned runtime transports with caller signing without
  copying backend-local `signEnvelope` / proto mapping logic.
- Preserve already-signed browser/user drafts. The signing decorator only signs
  drafts without `caller_signature`.

## Verification

- `go test ./...` under `sdk/go`.
- `go test -tags=easynet_direct_runtime ./...` under `sdk/go`.
- `PYTHONPATH=sdk/python python -m pytest sdk/python/tests/test_invocation.py`.
- `bash tools/scripts/check-sdk-cutover-readiness.sh`.

## Remaining Work

- Switch EasyNet backend from `internal/daemon_grpc` to the public Go SDK
  Runtime Core boundary wrapped by `RuntimeSigningTransport` where caller
  signing is required.
- Delete backend generated `internal/pb/axon/v1` ownership after all product
  routes consume SDK-owned profiles.
- Align Python concrete daemon transport with Go after backend cutover.
