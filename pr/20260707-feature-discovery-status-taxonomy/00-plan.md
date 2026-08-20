# Feature Discovery Status Taxonomy Plan

## Objective

Converge Runtime Core feature discovery on the four-state capability taxonomy
required by `docs/spec/daemon-sdk-requirements-v1.md`.

## Current Defect

The C ABI feature catalog and shared fixture expose bespoke profile status
strings such as `partial`, `cabi_core`, `carrier_partial`, and
`fetch_projection_partial`. Those strings duplicate detail already represented
by symbols, conformance evidence, and the SDK parity matrix, and they violate
the SPEC rule that capabilities exist only as `unsupported`, `seam`,
`provider-backed`, or `cutover-ready`.

## Implementation Steps

1. Replace profile status strings in the C ABI feature catalog with canonical
   SPEC states.
2. Update the shared feature-discovery fixture and schema to accept only the
   four canonical states.
3. Update Go/Python/Node/Java/Swift tests and conformance case expectations
   that pin the old bespoke status labels.
4. Add static guard coverage so new feature-discovery profiles cannot use
   non-SPEC status strings.
5. Run focused SDK tests and the aggregate SDK gates.
