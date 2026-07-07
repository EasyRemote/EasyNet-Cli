# Section 27 Tail Coverage Plan

## Objective

Complete the Section 27 conformance coverage gate for the tail SPEC cases:

- `memc/profile_exclusivity`
- `memc/consumer_coverage`
- `memc/semantic_alignment`
- `memc/no_core_bloat`
- `invocation/descriptor_ref_helper_delegation`

## Current Defect

The Section 27 coverage manifest and checker cover cases 1-38 but omit the
MEMC semantic-alignment tail and descriptor-ref delegation case. That makes the
coverage gate weaker than the normative SPEC list even though most of the
underlying conformance cases already exist.

## Steps

1. Add the missing shared `memc/semantic_alignment` conformance case.
2. Extend the Section 27 coverage manifest and checker required-case list for
   cases 39-43.
3. Wire Go/Python conformance assertions to the new shared case.
4. Update scaffold and runner reports so the conformance manifest is closed.
5. Run targeted Section 27, Go/Python, scaffold, and aggregate SDK gates.
