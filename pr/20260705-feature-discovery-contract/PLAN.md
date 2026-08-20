# Feature Discovery Contract Plan

## Goal

Make Runtime Core feature discovery a schema-backed shared SDK contract instead
of letting Rust, Go, and Python maintain separate feature payload shapes.

## Boundary Proof

- SDK-owned:
  - ABI root version and feature discovery DTO.
  - Runtime Core feature/profile capability matrix.
  - Shared schema and fixture used by P0 facade conformance tests.
- Product-owned:
  - Backend route cutover, EasyRemote decorators, and product HTTP/API policy.
  - Daemon runtime readiness values returned by a live process.

## Invariants

1. The daemon SDK SPEC remains unchanged.
2. `easynet_feature_discovery` stays ABI v4 and returns the same public fields:
   `abi_version`, `sdk_version`, `profiles`, `symbols`, and `axon_pb`.
3. Go and Python conformance tests must consume the same fixture payload instead
   of reconstructing a smaller language-local JSON object.
4. The fixture must be bound to a JSON schema and closed by scaffold checks.
5. The Rust feature catalog must live behind a named ABI-root object boundary,
   not inside the top-level `ffi::mod` export function body.

## Implementation Steps

1. Extract Rust feature discovery catalog construction into `src/ffi/features.rs`.
2. Add `feature-discovery.schema.json` and `feature-discovery.v4.json`.
3. Bind the fixture to the schema and reference it from Runtime Core version
   conformance cases.
4. Make Go/Python conformance helpers load the shared fixture and override only
   the ABI version for incompatible-case simulation.
5. Run Rust unit tests, Go/Python focused tests, runner gates, and scaffold.

## Verification

- `cargo test --lib ffi::`
- `cargo test --bin sdk-conformance-runner`
- `cargo run --bin sdk-conformance-runner -- --language go --adapter-report sdk/conformance/runner/go-action-adapter-report.json --format json`
- `cargo run --bin sdk-conformance-runner -- --language python --adapter-report sdk/conformance/runner/python-action-adapter-report.json --format json`
- `cd sdk/go && go test ./...`
- `cd sdk/python && python -m pytest`
- `bash tools/scripts/check-sdk-scaffold.sh`
