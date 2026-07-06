# Direct Runtime Identity Projection

## Objective

Move the Go direct daemon runtime transport closer to the Daemon SDK facade
target: direct runtime may translate SDK Invocation drafts into daemon Axon
gRPC frames, but DescriptorRef grammar and ability URA projection must stay
behind the Directory + Identity profile boundary.

## Boundary Proof

- Axon owns DescriptorRef grammar and canonical ability URA projection.
- EasyNet-Cli SDK direct runtime owns daemon UDS transport, request projection,
  stream/bidi lifecycle, and typed Runtime Core errors.
- Go facade code must not derive ability URA by parsing descriptor strings when
  an IdentityClient is available.
- The direct runtime can still carry Axon protobufs internally because it is a
  concrete daemon transport adapter, not a public product API.

## Invariants

1. Unary, stream, and bidi direct runtime paths use the same descriptor
   projection strategy.
2. DescriptorRef projection errors fail closed before daemon dispatch.
3. Complete Invocation tuple fields remain preserved in the daemon request.
4. Existing handle prepare/submit delegation remains unchanged.
5. No new DescriptorRef grammar or string parser is introduced in Go facade code.

## Verification

- `cd sdk/go && go test -tags easynet_direct_runtime ./...`
- `cd sdk/go && go test ./...`
- `cd sdk/go && CGO_ENABLED=1 go test -tags easynet_cabi ./...`
- `bash tools/scripts/check-sdk-cutover-readiness.sh`
- `git diff --check`
