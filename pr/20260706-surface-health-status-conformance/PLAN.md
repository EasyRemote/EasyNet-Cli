# Surface Health Status Conformance Plan

## Objective

Promote Surface health/status from local facade coverage to shared
provider-backed conformance evidence.

Go and Python already expose `SurfaceHealth` and `SurfaceStatus` readiness
projections plus Runtime/C ABI-backed health execution. The shared
`surface/page_carriers` case still covered only page carrier and manifest
projection behavior, leaving `SDK_PARITY.md` with a stale note that full surface
status remained schema/conformance declaration only.

## Invariants

- The SPEC remains unchanged.
- Backend rendering and browser auth stay product-owned; the SDK owns daemon
  Surface carriers and readiness DTO projection only.
- `SurfaceStatus` remains an alias over daemon-governed `SurfaceHealth`, not a
  backend page-route status model.
- Health/status requests must preserve the complete Invocation carrier context.
- Go and Python shared conformance must execute the same fixture evidence.

## Implementation Steps

1. Add shared `surface-health-request` and `surface-health-invocation`
   fixtures plus schema bindings.
2. Extend `surface/page_carriers` with health/status actions and expectations.
3. Execute `BuildHealthInvocation`, `SurfaceHealth`, and `SurfaceStatus` in Go
   shared conformance.
4. Execute `build_health_invocation`, `surface_health`, and `surface_status` in
   Python shared conformance.
5. Remove the stale SDK parity note that full surface status lacks conformance.

## Boundary Proof

Surface health/status remains SDK-owned because it is a daemon readiness
projection over `pages.health`. The SDK does not render pages, authenticate
browser routes, own CDN/cache policy, or expose backend public-route status.
The request fixture carries caller, callee, subject, nonce, descriptor version,
and causal context; the invocation fixture proves the carrier lowers to a
complete daemon ability invocation.

## Verification Plan

- `go test ./... -run TestGoSurfaceFacadeExecutesSharedPageCarrierConformanceCase`
- `uv run python -m unittest tests.test_conformance.SharedConformanceFixtureTests.test_python_surface_executes_shared_page_carrier_conformance_case`
- `go test ./...`
- `uv run python -m unittest discover tests`
- `cargo test --bin sdk-conformance-runner`
- Go/Python `sdk-conformance-runner` adapter reports
- `bash tools/scripts/check-sdk-scaffold.sh`
- `git diff --check`
- `git diff -- docs/spec/daemon-sdk-requirements-v1.md`

## Verification Result

- PASS: `go test ./... -run TestGoSurfaceFacadeExecutesSharedPageCarrierConformanceCase`
- PASS: `uv run python -m unittest tests.test_conformance.SharedConformanceFixtureTests.test_python_surface_executes_shared_page_carrier_conformance_case`
- PASS: `go test ./...`
- PASS: `uv run python -m unittest discover tests`
- PASS: `cargo test --bin sdk-conformance-runner`
- PASS: `cargo run --bin sdk-conformance-runner -- --language go --adapter-report sdk/conformance/runner/go-action-adapter-report.json --format jsonl`
- PASS: `cargo run --bin sdk-conformance-runner -- --language python --adapter-report sdk/conformance/runner/python-action-adapter-report.json --format jsonl`
- PASS: `bash tools/scripts/check-sdk-scaffold.sh`
- PASS: `git diff --check`
- PASS: `git diff -- docs/spec/daemon-sdk-requirements-v1.md` produced no diff.
