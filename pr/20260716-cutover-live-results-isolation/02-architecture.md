# Architecture

## Owner Boundaries

- `check-sdk-cutover-readiness.sh` owns release-gate orchestration.
- `check-sdk-conformance-reports.sh` owns live conformance artifact generation.
- `check-sdk-parity-matrix.sh` owns parity validation over a supplied result
  directory.

## Layering

The cutover gate should pass an explicit artifact boundary between producer and
consumer:

```text
cutover readiness
  -> conformance reports writes run-scoped live results
  -> parity matrix validates the same run-scoped live results
```

The default `target/sdk-conformance-live-results` path remains useful for
manual commands, but release orchestration must not treat it as canonical state.

## Obsolete Path Removed

This slice removes the cutover script's direct dependency on the shared default
live-result directory for the release parity step.
