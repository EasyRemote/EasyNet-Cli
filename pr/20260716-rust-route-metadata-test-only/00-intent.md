# Rust Route Metadata Test-Only Boundary

## Goal

Keep generated Rust route modules focused on runtime ability names by making
manifest profile and digest metadata test-only.

## Concrete Use Case

Fresh cutover builds repeatedly reported unused generated constants such as
`PRINCIPAL_LIFECYCLE_PROFILE`, `*_ROUTE_MANIFEST_SHA256`, `RECEIPT_PROFILE`,
and `RUNTIME_ADMIN_PROFILE`. `rg` and CodeGraph-style lookup show ability-name
constants are consumed by runtime/admission/conformance paths, while profile and
digest constants are consumed only by tests that verify generator freshness.

## Non-Goals

- Do not remove route-manifest freshness tests.
- Do not change generated Go or Python provider route modules.
- Do not change ability names or runtime dispatch behavior.
- Do not touch unrelated dirty working-tree files.

## Acceptance Criteria

1. Rust generated route profile/hash constants are emitted under `#[cfg(test)]`.
2. Runtime ability constants remain available in normal builds.
3. Generated files are reproducible from `provider_routes/route_generator.py`.
4. Focused route tests and generator checks pass.
