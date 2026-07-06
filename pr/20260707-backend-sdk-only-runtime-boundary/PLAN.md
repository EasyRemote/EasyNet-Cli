# Backend SDK-Only Runtime Boundary

## Intent

Converge the EasyNet backend onto the public Go Daemon SDK Runtime Core
boundary. Backend product code must stop owning raw daemon gRPC, generated
`axon.v1` protobuf packages, daemon socket discovery, canonical Invocation
bytes, and bidi protocol frame mapping.

## Boundary Invariants

- Backend may consume `easynet.run/cli/sdk/go` public facades only.
- Backend must not import or define generated `axon.v1` protobuf packages.
- Backend must not own daemon socket path resolution or raw gRPC dial state.
- Runtime Core lifecycle remains explicit: complete draft, prepare, sign,
  submit, await, stream, bidi, and terminal observations.
- If a concrete direct daemon provider is used, it must be an SDK-owned provider
  seam. Backend must not recreate that provider in `internal/`.

## Current Violations

- `internal/daemon_grpc` owns direct daemon gRPC transport and protobuf mapping.
- `internal/pb/axon/v1` stores backend-local generated Axon protocol packages.
- `internal/svc` adapts SDK Runtime Core back into backend-local Axon structs,
  preserving backend ownership of canonical bytes and bidi frames.

## Refactor Strategy

1. Remove product-layer references to direct daemon transport from the highest
   call sites first.
2. Move protocol-shaped conversions behind SDK-owned DTOs or delete them when a
   public SDK profile already exists.
3. Keep temporary states explicit as `Seam` or `Provider-backed`; do not label
   backend cutover-ready until the static boundary checker passes.

## Verification

- `go test ./...` in `EasyNet-Cli/sdk/go`.
- Focused backend package tests for touched route/profile family.
- `tools/scripts/check-backend-sdk-only-boundary.sh ../EasyNet/backend`.
- Backend `go test ./...` when the slice touches shared service wiring.
