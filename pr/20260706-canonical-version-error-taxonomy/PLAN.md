# Canonical Version Error Taxonomy Plan

## Goal

Converge ABI/control version mismatch failures onto the SPEC section 22
canonical RuntimeError code `VERSION_MISMATCH` across Go, Python, and shared
conformance evidence.

## Root Problem

The SDK normalization layer already maps `VERSION_INCOMPATIBLE` to
`VERSION_MISMATCH`, but Go C ABI transports, Go `RequireABI`, Python C ABI,
Python control IPC, Python `Client.require_abi`, and conformance expectations
still construct or expect the retired `VERSION_INCOMPATIBLE` vocabulary.

## Boundary Proof

- Public legacy constants remain available for source compatibility.
- `NormalizeErrorCode` / `normalize_error_code` continue accepting
  `VERSION_INCOMPATIBLE`.
- SDK-created version failures use `VERSION_MISMATCH`.
- Conformance expectations assert canonical output while old aliases remain
  accepted as inputs.
- No SPEC edits.

## Implementation Order

1. Migrate Go version-mismatch constructors to `ErrVersionMismatch`.
2. Migrate Python version-mismatch constructors to `ErrorCode.VERSION_MISMATCH`.
3. Update shared conformance expectations and language tests to assert canonical
   output while preserving legacy alias matching where useful.
4. Run Go/Python, scaffold, and conformance gates.

## Verification

- `(cd sdk/go && go test ./...)`
- `(cd sdk/python && uv run python -m unittest discover tests)`
- `cargo test --bin sdk-conformance-runner`
- `bash tools/scripts/check-sdk-scaffold.sh`
- Four-language adapter reports through `sdk-conformance-runner` for Rust,
  C ABI, Go, and Python.
- `git diff --check`
