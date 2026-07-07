# Java/Swift Surface Seam Plan

## Goal

Converge Java and Swift P1 facades with the shared Surface profile seam from the
daemon SDK SPEC.

This iteration covers:

- `surface/page_carriers`

## Scope

- Add Java and Swift Surface profile DTOs for request carriers, page records,
  page pages, manifests, public page refs, mutation results, and health/status
  projections.
- Add `SurfaceClient` and `SurfaceTransport` seams over injected transports.
- Build complete Invocation carriers through injected transports; facades do not
  concatenate descriptor refs.
- Keep rendering and public HTTP routing outside the SDK.
- Keep page identity as daemon-governed `surface_ref` and page ids, not backend
  rows or filesystem transport.
- Update conformance reports, scaffold checks, and Java/Swift status docs.

## Non-Goals

- No provider-backed Java/Swift daemon transport.
- No backend rendering, HTTP routing, or browser authorization.
- No direct filesystem page transport.
- No SDK-owned page publication policy.

## Verification

- `tools/scripts/check-java-sdk-seam.sh`
- `tools/scripts/check-swift-sdk-seam.sh`
- `tools/scripts/check-sdk-conformance-reports.sh`
- `tools/scripts/check-sdk-scaffold.sh`
- `tools/scripts/check-sdk-ura-naming.sh`
- `tools/scripts/check-sdk-package-metadata.sh`
- `git diff --check`
