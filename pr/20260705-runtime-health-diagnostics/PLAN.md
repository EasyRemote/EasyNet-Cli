# Runtime Health Diagnostics Plan

## Goal

Bring the Health profile closer to the SPEC object model by making Rust C ABI
runtime health emit the shared `RuntimeHealth` DTO directly and by adding the
missing `DiagnosticsReport` operation across C ABI, Go, and Python facades.

## Boundary Proof

- SDK-owned:
  - Runtime health readiness projection.
  - Diagnostics report DTO and health profile operation.
  - C ABI symbol projection for Health profile.
  - Go/Python facade methods over the same transport contract.
- Product-owned:
  - Product dashboards, UI rendering, alerting policy, and route-level health
    presentation.
  - Backend HTTP auth and public API response shaping.

## Invariants

1. The SPEC remains unchanged.
2. C ABI `easynet_runtime_health` must return fields accepted by
   `sdk/schemas/health.schema.json`; language facades must not be the source of
   health shape normalization.
3. `DiagnosticsReport` is a read-only Health profile DTO; it must not become
   product monitoring or backend UI state.
4. Go and Python expose the same health operations: `runtime_health`/readiness
   and diagnostics.
5. Diagnostics must distinguish API, daemon, invocation, directory, trust, and
   runtime readiness checks without collapsing degraded readiness into success.

## Implementation Steps

1. Add `diagnostics.schema.json` and `diagnostics.ready.v4.json`.
2. Add C ABI `easynet_runtime_diagnostics` and make runtime health return the
   shared schema shape.
3. Extend Go/Python Health transport and clients with typed diagnostics DTOs.
4. Bind diagnostics fixtures into conformance cases and scaffold checks.
5. Run Rust, Go, Python, scaffold, and adapter-report gates.

## Verification

- `cargo test --lib runtime_health`
- `cargo test --lib runtime_diagnostics`
- `cargo test --bin sdk-conformance-runner`
- `cd sdk/go && go test ./...`
- `cd sdk/python && python -m pytest tests/test_health.py tests/test_cabi.py tests/test_conformance.py`
- `bash tools/scripts/check-sdk-scaffold.sh`
- P0 adapter reports for Rust, C ABI, Go, and Python.
