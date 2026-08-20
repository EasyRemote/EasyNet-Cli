# Java/Swift Publication Seam Plan

## Goal

Converge Java and Swift P1 facades with the shared Publication profile seam from
the daemon SDK SPEC.

This iteration covers:

- `publication/resource_carriers`

## Scope

- Add Java and Swift Publication profile DTOs for local resource-ref requests,
  daemon-authored `ResourceRef` projections, package validation inputs and
  projections, complete deploy carriers, and unpublish carriers.
- Add `PublicationClient` and `PublicationTransport` seams over injected
  transports.
- Preserve complete Invocation tuple fields in deploy/unpublish requests.
- Keep package inspection, Python decorators, host process runtime, plugin
  lifecycle policy, and product catalog state outside Java/Swift seams.
- Update conformance reports, scaffold checks, and Java/Swift status docs.

## Non-Goals

- No provider-backed Java/Swift daemon transport.
- No plugin install/list/show/enable/disable runtime implementation.
- No local package scanning or manifest hashing inside the SDK seam.
- No EasyRemote product abstractions.

## Verification

- `tools/scripts/check-java-sdk-seam.sh`
- `tools/scripts/check-swift-sdk-seam.sh`
- `tools/scripts/check-sdk-conformance-reports.sh`
- `tools/scripts/check-sdk-scaffold.sh`
- `tools/scripts/check-sdk-ura-naming.sh`
- `tools/scripts/check-sdk-package-metadata.sh`
- `git diff --check`
