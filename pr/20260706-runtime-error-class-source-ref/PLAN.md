# Runtime Error Class and Source Ref Parity

## Objective

Converge Go and Python Runtime Core typed errors so both P0 facades expose the
same language-side error class projection and stable profile/package source
reference accessors while preserving the canonical daemon error JSON schema.

## Boundary Proof

- Axon and the daemon remain the source of truth for canonical error codes and
  protocol failure semantics.
- SDK facades derive coarse language-side classes from existing canonical
  `ErrorCode` values only; they do not create new wire codes or admission rules.
- Profile source refs are package-level diagnostic projections attached to SDK
  profile errors; they do not alter the shared error schema or Invocation tuple.
- Existing caller-provided error details must be preserved when profile defaults
  are attached.

## Implementation Steps

1. Add Go `ErrorClass`, `ErrorClassForCode`, and SDKError profile/source
   accessors.
2. Add Python `ErrorClass`, `error_class_for_code`, and SDKError
   profile/source accessors.
3. Extend the shared `error/profile_source_refs` conformance case and make Go
   and Python verify the new expectations.
4. Update parity docs so typed errors no longer claim missing P0 language error
   classes/source refs.
5. Run targeted Go/Python error and conformance tests plus parity gates.

## Verification

- `go test ./... -run 'Error|Conformance'`
- `PYTHONPATH=tests ./.venv/bin/python -m unittest tests.test_errors tests.test_conformance`
- `tools/scripts/check-sdk-parity-matrix.sh --self-test`
- `bash tools/scripts/check-sdk-scaffold.sh`
- `cargo run --bin sdk-conformance-runner -- --language go --adapter-report sdk/conformance/runner/go-action-adapter-report.json`
- `cargo run --bin sdk-conformance-runner -- --language python --adapter-report sdk/conformance/runner/python-action-adapter-report.json`
- `git diff --check`

## Remaining After This Slice

- Product repository cutover gates remain outside this Runtime Core typed error
  slice.
- Non-P0 language packages still need implementation/reporting before the full
  SDK family can claim language-wide parity.
