# Typed Lifecycle Errors Plan

## Goal

Make ABI root and daemon lifecycle failures record schema-backed typed error
codes instead of falling back to legacy untyped last-error strings.

## Boundary Proof

- Runtime Core owns ABI feature discovery, daemon lifecycle, attach/discover,
  open-client, shutdown, and typed error projection.
- This slice does not change Invocation, stream, bidi, or profile carrier
  behavior.
- C ABI integer return codes remain the primary C control signal; typed JSON is
  the language-facade projection over the same return code.

## Invariants

1. The SPEC remains unchanged.
2. Every changed failure branch must call `set_last_error_code` with the same
   code it returns.
3. No language facade should need to parse `easynet_last_error()` text for ABI
   root or daemon lifecycle errors.
4. Invalid UTF-8 must be classified as `ERR_INVALID_UTF8`, not null pointer or
   generic failure.
5. Existing success paths must still clear the last-error slot.

## Implementation Steps

1. Update ABI root errors in `src/ffi/mod.rs`.
2. Update daemon lifecycle errors in `src/ffi/daemon/mod.rs`.
3. Add focused tests that `easynet_last_error_json` reflects daemon lifecycle
   return codes.
4. Run Rust, SDK scaffold, and conformance gates.

## Verification

- `cargo fmt`
- `cargo test --lib ffi::daemon`
- `cargo test --lib ffi::errors`
- `cargo test --bin sdk-conformance-runner`
- `bash tools/scripts/check-sdk-scaffold.sh`
- URA terminology scan for touched files and plan.
