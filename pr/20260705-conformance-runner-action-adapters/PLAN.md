# Conformance Runner Action Adapter Plan

## Goal

Move the conformance runner from manifest-only seam toward a provider-backed
runner contract by validating language action-adapter result records against the
shared case manifest.

## Boundary Proof

- SDK-owned:
  - Shared case and fixture graph validation.
  - Machine-readable result record shape.
  - Language action-adapter coverage validation for required cases.
  - Evidence references tying adapter records to language conformance tests.
- Product-owned:
  - Real daemon deployment, backend route execution, browser/API policy, and
    EasyRemote product repository cutover.
  - Non-SDK smoke tests that prove external repositories have removed lower-layer
    imports.

## State Model

Each required case reaches one deterministic runner result:

- `passed`: case manifest is valid and the language adapter report contains a
  passed record with evidence.
- `failed`: manifest is invalid, adapter report is missing the required case, or
  the adapter record reports failure.
- `skipped`: the case is not required for the requested language.

## Implementation Steps

1. Add an optional runner `--adapter-report` input.
2. Define adapter-report DTOs and validate language/profile/case/evidence
   consistency.
3. Add Go and Python adapter reports covering their full required case sets.
4. Update runner docs, parity matrix, and parity docs.
5. Verify with runner commands, Go/Python tests, scaffold, formatting, and diff
   hygiene.

## Verification

- `cargo test --bin sdk-conformance-runner`
- `cargo run --bin sdk-conformance-runner -- --language go --adapter-report sdk/conformance/runner/go-action-adapter-report.json --format json`
- `cargo run --bin sdk-conformance-runner -- --language python --adapter-report sdk/conformance/runner/python-action-adapter-report.json --format json`
- `go test -count=1 ./...` in `sdk/go`
- `uv run --project sdk/python python -m unittest discover -s sdk/python/tests`
- `bash tools/scripts/check-sdk-parity-matrix.sh --self-test`
- `bash tools/scripts/check-sdk-scaffold.sh`
