# Typed Invocation Preflight Errors Plan

## Goal

Make deterministic Invocation C ABI preflight failures record schema-backed
typed last-error records instead of plain untyped strings.

## Boundary Proof

- This slice is Runtime Core C ABI preflight only: null output pointers,
  invalid local handles, invalid UTF-8 inputs, unavailable feature symbols,
  builder registry failures, builder validation failures, and local allocation
  failures.
- Daemon runtime/admission/ability responses remain a separate slice because
  they need receipt/invocation references and runtime-stage classification.
- Public C return codes and exported ABI symbols remain unchanged.

## Invariants

1. The SPEC remains unchanged.
2. Every changed branch must write `set_last_error_code(code, message)` with
   the same `code` it returns.
3. No changed branch may fall back to `GENERIC` unless the returned integer is
   `ERR_GENERIC`.
4. Success paths keep clearing last-error records.
5. Language facades must be able to use `easynet_last_error_json` after
   preflight failure without parsing `easynet_last_error()` text.

## Implementation Steps

1. Add local `record_invocation_error` helpers in `src/ffi/invocation/mod.rs`.
2. Convert exported Invocation preflight checks and shared builder helpers.
3. Add focused tests for typed invalid-handle and builder-validation records.
4. Run Rust and SDK conformance gates.

## Verification

- `cargo fmt`
- `cargo test --lib ffi::invocation`
- `cargo test --lib ffi::errors`
- `cargo test --bin sdk-conformance-runner`
- `bash tools/scripts/check-sdk-scaffold.sh`
- Four-language adapter report gate
- URA terminology scan for touched files and plan.
