# Canonical Error Code Projection Plan

## Goal

Align C ABI typed error JSON with the Daemon SDK section 22 RuntimeError code
vocabulary while preserving stable integer ABI codes and `details.abi_symbol`.

## Boundary Proof

- This slice changes only the JSON `code` projection for existing ABI integers.
- It does not rename or renumber exported `ERR_*` constants.
- It does not change Go/Python wire-code normalization compatibility inputs.
- It converges Go profile runtime terminal-failure defaults to canonical
  RuntimeError codes.
- It does not change the SPEC.
- `details.abi_code` and `details.abi_symbol` remain the audit trail for C ABI
  compatibility.

## Canonical Mapping

- `ERR_DAEMON_DOWN` -> `DAEMON_OFFLINE`
- `ERR_VERSION_INCOMPATIBLE` -> `VERSION_MISMATCH`
- `ERR_ABILITY_FAILED` -> `ADMISSION_DENIED`
- `ERR_NOT_FOUND` -> `ABILITY_NOT_FOUND`
- `ERR_PROTOCOL` -> `PROTOCOL_MISMATCH`

## Invariants

1. Public integer ABI return codes remain unchanged.
2. Bindings can still inspect `details.abi_symbol` without parsing messages.
3. The Go/Python SDK canonical constants remain the public RuntimeError shape.
4. Existing legacy wire aliases stay accepted by normalization functions until
   downstream callers finish migrating.

## Verification

- `cargo fmt`
- `cargo test --lib ffi::errors`
- `cargo test --lib ffi::invocation`
- `(cd sdk/go && go test ./...)`
- `(cd sdk/python && uv run python -m unittest discover tests)`
- `cargo test --bin sdk-conformance-runner`
- `bash tools/scripts/check-sdk-scaffold.sh`
- `cargo check --lib --features axon-pb`
- `cargo run --bin sdk-conformance-runner -- --language rust --adapter-report sdk/conformance/runner/rust-action-adapter-report.json --format jsonl`
- `cargo run --bin sdk-conformance-runner -- --language c_abi --adapter-report sdk/conformance/runner/c-abi-action-adapter-report.json --format jsonl`
- `cargo run --bin sdk-conformance-runner -- --language go --adapter-report sdk/conformance/runner/go-action-adapter-report.json --format jsonl`
- `cargo run --bin sdk-conformance-runner -- --language python --adapter-report sdk/conformance/runner/python-action-adapter-report.json --format jsonl`
