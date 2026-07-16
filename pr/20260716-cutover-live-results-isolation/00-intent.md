# Cutover Live Results Isolation

## Goal

Make `check-sdk-cutover-readiness.sh` consume a run-scoped SDK conformance live
results directory instead of the mutable default
`target/sdk-conformance-live-results` path.

## Concrete Use Case

Developers often run language slices such as
`SDK_CONFORMANCE_LANGUAGES=rust check-sdk-conformance-reports.sh`. That leaves a
partial live-result directory on disk. The release cutover gate should not
depend on whether a previous local slice left stale or incomplete artifacts at
the default path.

## Non-Goals

- Do not change conformance case semantics.
- Do not commit generated live-result JSON under `target/`.
- Do not weaken source-tree, run-nonce, toolchain, or Axon-revision
  attestations.

## Acceptance Criteria

1. Cutover readiness creates one explicit result directory for its own run.
2. The conformance producer writes to that directory.
3. The parity consumer validates that same directory.
4. Existing external callers of `check-sdk-conformance-reports.sh` keep their
   current defaults.
