# Node Health Seam Intent

Implement the Node/TypeScript Health profile seam described by
`docs/spec/daemon-sdk-requirements-v1.md`.

## Scope

- Add a generic `HealthClient` over an injected health transport.
- Decode shared `health.schema.json` and `diagnostics.schema.json` DTOs.
- Preserve the distinction between API liveness and runtime readiness.
- Declare Node evidence for the shared `health/api_vs_runtime` conformance case.

## Out Of Scope

- No daemon lifecycle provider.
- No C ABI bridge.
- No direct daemon socket, HTTP, or Axon transport provider.
- No product-specific health route, backend, EasyRemote, or UI behavior.
