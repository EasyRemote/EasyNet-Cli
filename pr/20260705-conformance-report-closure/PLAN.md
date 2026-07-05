# Conformance Report Closure Plan

## Goal

Make SDK action-adapter reports closed over the shared conformance manifest so
language parity evidence cannot contain stale, misspelled, or undeclared case
records that the runner silently ignores.

## Boundary Proof

- SDK-owned:
  - Manifest case identity and profile graph.
  - Adapter report validation.
  - Deterministic failed records for missing or invalid evidence.
- Product-owned:
  - Backend and EasyRemote cutover smokes.
  - Product repository import deletion evidence.

## State Model

Each adapter report record must resolve to exactly one manifest case:

- Known and required for the requested language: eligible for pass/fail
  adapter projection.
- Known but not required for the requested language: report contract failure.
- Unknown to the manifest: report contract failure.

## Implementation Steps

1. Build a manifest case index before adapter report validation.
2. Validate every adapter report record against the manifest index.
3. Fail fast for unknown or language-undeclared adapter records.
4. Add unit tests for both failure modes.
5. Update runner documentation and scaffold literals.

## Verification

- `cargo test --bin sdk-conformance-runner`
- `cargo run --bin sdk-conformance-runner -- --language go --adapter-report sdk/conformance/runner/go-action-adapter-report.json --format json`
- `cargo run --bin sdk-conformance-runner -- --language python --adapter-report sdk/conformance/runner/python-action-adapter-report.json --format json`
- `bash tools/scripts/check-sdk-scaffold.sh`
- `git diff --check`
- forbidden address-spelling scan over touched files
