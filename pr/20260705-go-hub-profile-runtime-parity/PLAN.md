# Go Hub Profile Runtime Parity Plan

## Goal

Converge Go Surface, Compatibility, and Wrappers from documented seams to
provider-backed SDK capabilities where the implementation already has explicit
Runtime Core-backed transports and tests.

## Boundary Proof

- SDK-owned:
  - Surface page/manifest/public-ref/health carriers and projections.
  - Compatibility model/chat/file carriers and OpenAI-style adapter projections.
  - Wrapper file/session carriers and record projections.
  - Runtime Core transport adapters that lower profile DTOs to complete
    Invocation drafts and project daemon outputs back into SDK DTOs.
- Product-owned:
  - Browser route rendering, auth gates, CDN/cache policy, content UX.
  - Product API-key policy, billing, quota, HTTP response shaping.
  - Backend storage policy, WebSocket/SSE route fanout, and concrete UI/session
    integration.

## Implementation Steps

1. Add parity validator requirements that provider-backed Go/Python
   hub-facing profiles carry Runtime-backed evidence.
2. Update the parity matrix evidence/status for Go Surface, Compatibility, and
   Wrappers.
3. Sync SDK parity documentation with the machine matrix.
4. Run parity checks, Go tests, Python tests, scaffold checks, formatting, and
   diff hygiene.

## Verification

- `tools/scripts/check-sdk-parity-matrix.sh`
- `tools/scripts/check-sdk-parity-matrix.sh --self-test`
- `go test -count=1 ./...` in `sdk/go`
- `PYTHONPATH=sdk/python python -m pytest sdk/python/tests -q`
- `tools/scripts/check-sdk-scaffold.sh`
- `cargo fmt --check`
- `git diff --check`
