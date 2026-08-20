# Typed Runtime Invocation Errors Plan

## Goal

Make daemon, transport, admission, timeout, cancellation, and protocol failures returned through the Invocation C ABI record typed last-error JSON instead of plain text records, while preserving exported integer return codes and public ABI symbols.

## Boundary Proof

- This slice is Runtime Core Invocation error classification only.
- It covers daemon transport errors, tonic status/admission errors, daemon unary response errors, and synchronous signed-submit terminal errors.
- It does not change the SDK error JSON schema, exported ABI constants, Axon status taxonomy, or daemon receipt URA construction.
- Receipt URA and invocation id remain null unless the daemon/Axon path provides authoritative values; this slice does not fabricate them.

## Invariants

1. The SPEC remains unchanged.
2. Every changed failure branch records `set_last_error_code(code, message)` through the local Invocation helper.
3. The recorded ABI code must match the returned ABI integer.
4. Transport, timeout, cancellation, permission, not-found, invalid-invocation, and protocol failures must remain distinguishable through `easynet_last_error_json`.
5. Daemon/Axon protocol semantics remain delegated; this is C ABI projection only.

## Implementation Steps

1. Replace runtime/admission `set_last_error` calls in `src/ffi/invocation/mod.rs` with typed helper calls.
2. Add focused tests for daemon transport/status and sync-submit terminal typed last-error projection.
3. Run focused Runtime Core gates and SDK conformance checks.

## Verification

- `cargo fmt`
- `cargo test --lib ffi::invocation`
- `cargo test --lib ffi::errors`
- `cargo test --bin sdk-conformance-runner`
- `bash tools/scripts/check-sdk-scaffold.sh`
- `cargo check --lib --features axon-pb`
- Four-language adapter report gate if the ABI projection changes remain broad enough to affect facade claims.
