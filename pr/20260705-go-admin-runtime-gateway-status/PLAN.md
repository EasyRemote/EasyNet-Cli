# Go Admin Runtime Gateway Status

## Goal

Implement Go Admin runtime GatewayStatus through an explicit daemon status provider seam while preserving the existing GatewayStatus DTO and avoiding Runtime invoke fabrication.

## Boundary Proof

- AdminRuntimeTransport remains the Admin + Gateway facade over Runtime Core for ability-backed admin operations.
- GatewayStatus is daemon lifecycle/status owned and is supplied by an explicit provider, not by an admin system ability Invocation.
- The Go facade validates the existing GatewayStatus projection shape before returning it to callers.
- C ABI admin status projection remains unchanged and continues to delegate to Rust-owned projection.
- No backend product presence registry, certificate authority policy, Hub auth, or browser session state is introduced.

## Invariants

- GatewayStatus without a configured provider fails closed with a typed SDK error.
- Provider output must decode as the existing GatewayStatus DTO.
- The AdminGatewayStatusRequest JSON remains unchanged.
- Runtime invocation carriers and lifecycle operations are untouched.
- No retired address terminology is introduced in touched files.

## Verification

- go test -count=1 ./... in sdk/go.
- go test -count=1 -tags easynet_cabi ./... in sdk/go.
- cargo fmt --check.
- bash tools/scripts/check-sdk-scaffold.sh.
- git diff --check.
- Retired address terminology scan over touched files.
