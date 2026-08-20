# Canonical Execution Error Taxonomy Plan

## Goal

Remove SDK-created `ABILITY_FAILED` errors from Python runtime/profile paths and
project execution failures into the SPEC section 22 canonical RuntimeError
vocabulary.

## Root Problem

Go already treats `ABILITY_FAILED` as a legacy alias for `ADMISSION_DENIED`, but
Python still constructs new `ABILITY_FAILED` errors in direct runtime,
publication, wrapper, and transport adapters. That lets a retired code leak
from one language implementation and breaks the single shared runtime model.

## Boundary Proof

- Public legacy constants remain available for source compatibility.
- `normalize_error_code` continues to accept `ABILITY_FAILED`.
- SDK-created execution failures use `ADMISSION_DENIED`.
- SDK-created cancellation failures use `CANCELLED`.
- Malformed or unknown remote error frames use `PROTOCOL_MISMATCH`.
- No SPEC edits.

## Implementation Order

1. Migrate Python runtime/profile constructors from `ABILITY_FAILED` to
   canonical codes based on terminal state and wire error shape.
2. Keep legacy `ABILITY_FAILED` only in constants, normalization aliases, and
   tests that prove old input normalization.
3. Update tests to assert canonical output rather than relying on alias
   matching.
4. Run focused Python tests, then full Go/Python and conformance gates.

## Verification

- `python3 -m compileall sdk/python/easynet_sdk`
- `(cd sdk/python && uv run python -m unittest discover tests)`
- `(cd sdk/go && go test ./...)`
- `cargo test --bin sdk-conformance-runner`
- `bash tools/scripts/check-sdk-scaffold.sh`
- Four-language adapter reports through `sdk-conformance-runner` for Rust,
  C ABI, Go, and Python.
- `git diff --check`
